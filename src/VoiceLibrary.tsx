import { Check, FolderOpen, Library, Mic, Pause, Play, Plus, Scissors, Trash2, Wand2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  createVoiceProfile,
  deleteVoiceProfile,
  localSpeechMediaUrl,
  setDefaultVoiceProfile,
  updateVoiceProfile,
} from "./api";
import {
  createVoiceReferenceExcerpt,
  MAX_EXCERPT_ANALYSIS_SECONDS,
  VOICE_EXCERPT_SECONDS,
} from "./voiceReferenceProcessing";
import type {
  CreateVoiceProfileRequest,
  VoiceLibrarySnapshot,
  VoicePerformance,
  VoiceProfile,
} from "./types";

const MIN_REFERENCE_SECONDS = 3;
const MAX_REFERENCE_SECONDS = 45;
const IDEAL_MIN_SECONDS = 8;
const IDEAL_MAX_SECONDS = 20;

interface PendingReference {
  blob: Blob;
  source: "recorded" | "imported";
  fileName: string;
  durationSeconds: number;
  previewUrl: string;
}

export function formatErrorMessage(cause: unknown): string {
  const raw = cause instanceof Error ? cause.message : String(cause ?? "");
  return raw
    .replace(/^(?:Error:\s*)+/i, "")
    .replace(/^voice library request is invalid:\s*/i, "")
    .trim() || "An unknown error occurred.";
}

export function diagnoseAudioDecodeError(fileOrBlob?: Blob | File): string {
  const name = fileOrBlob && "name" in fileOrBlob ? (fileOrBlob as File).name.toLowerCase() : "";
  const type = fileOrBlob?.type?.toLowerCase() || "";

  if (name.endsWith(".m4a") || type.includes("mp4") || type.includes("m4a")) {
    return "Could not decode this M4A file. M4A is a container and its audio codec may not be supported by this desktop WebView; convert it to PCM WAV or MP3.";
  }
  if (name.endsWith(".opus") || type.includes("opus")) {
    return "Could not decode this .opus file. Ensure it is formatted as Ogg Opus or WebM Opus, or convert it to WAV, MP3, or FLAC.";
  }
  if (name.endsWith(".flac") || type.includes("flac")) {
    return "Could not decode this FLAC file. Check that it is not damaged, or convert it to PCM WAV or MP3.";
  }
  if (name.endsWith(".wav") || type.includes("wav") || type.includes("wave")) {
    return "Could not decode this WAV file. Ensure it is a standard PCM WAV recording.";
  }
  if (name.endsWith(".mp3") || type.includes("mpeg") || type.includes("mp3")) {
    return "Could not decode this .mp3 file. The file may be damaged or protected. Try converting it to standard WAV or MP3.";
  }
  return "This audio file could not be decoded. Use a clean WAV, MP3, FLAC, Ogg/Opus, WebM, or AAC M4A recording.";
}

function blobAsBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Kestrel could not read the voice recording."));
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const separator = result.indexOf(",");
      if (separator < 0) reject(new Error("Kestrel could not encode the voice recording."));
      else resolve(result.slice(separator + 1));
    };
    reader.readAsDataURL(blob);
  });
}

function audioDuration(blob: Blob): Promise<number> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const audio = new Audio();
    let settled = false;
    let timeout: number | null = null;
    const finish = (complete: () => void) => {
      if (settled) return;
      settled = true;
      if (timeout !== null) window.clearTimeout(timeout);
      audio.onloadedmetadata = null;
      audio.onerror = null;
      URL.revokeObjectURL(url);
      complete();
    };
    timeout = window.setTimeout(() => {
      finish(() => reject(new Error("Kestrel timed out while reading this recording. The file may be damaged or use an unsupported audio codec.")));
    }, 15_000);
    audio.onloadedmetadata = () => {
      const duration = audio.duration;
      if (!Number.isFinite(duration) || duration <= 0) {
        finish(() => reject(new Error("Kestrel could not measure the duration of this recording. The file may be empty or corrupted.")));
      } else {
        finish(() => resolve(duration));
      }
    };
    audio.onerror = () => {
      finish(() => reject(new Error(diagnoseAudioDecodeError(blob))));
    };
    audio.preload = "metadata";
    audio.src = url;
  });
}

export function assessVoiceReference(duration: number): { tone: string; text: string } {
  if (duration < MIN_REFERENCE_SECONDS) {
    return { tone: "bad", text: `Too short (${duration.toFixed(1)}s). Record or import at least ${MIN_REFERENCE_SECONDS} seconds of clear speech.` };
  }
  if (duration > MAX_REFERENCE_SECONDS) {
    const nextStep = duration <= MAX_EXCERPT_ANALYSIS_SECONDS
      ? "Use the excerpt controls below."
      : "Use an audio editor to extract one clean 8–20 second passage.";
    return { tone: "bad", text: `Too long (${duration.toFixed(1)}s). ${nextStep}` };
  }
  if (duration < IDEAL_MIN_SECONDS) {
    return { tone: "warn", text: "Usable, but 8–20 seconds usually preserves the voice more reliably." };
  }
  if (duration > IDEAL_MAX_SECONDS) {
    return { tone: "warn", text: "Usable. A clean 8–20 second excerpt is usually faster and more consistent." };
  }
  return { tone: "good", text: "Good reference length. Listen once for music, echo, or another speaker before saving." };
}

export function VoiceLibraryDialog({
  snapshot,
  onSnapshot,
  onClose,
}: {
  snapshot: VoiceLibrarySnapshot;
  onSnapshot: (snapshot: VoiceLibrarySnapshot) => void;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const timerRef = useRef<number | null>(null);
  const startedRef = useRef(0);
  const operationRef = useRef(0);
  const pendingRef = useRef<PendingReference | null>(null);
  const pendingAudioRef = useRef<HTMLAudioElement | null>(null);
  const [adding, setAdding] = useState(false);
  const [recording, setRecording] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const [pending, setPending] = useState<PendingReference | null>(null);
  const [name, setName] = useState("");
  const [language, setLanguage] = useState("Auto");
  const [tags, setTags] = useState("");
  const [performance, setPerformance] = useState<VoicePerformance>("natural");
  const [consent, setConsent] = useState(false);
  const [findingExcerpt, setFindingExcerpt] = useState(false);
  const [excerptNotice, setExcerptNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog || dialog.open) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
  }, []);

  const stopTracks = () => {
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
    if (timerRef.current !== null) window.clearInterval(timerRef.current);
    timerRef.current = null;
  };

  useEffect(() => () => {
    operationRef.current++;
    if (pendingRef.current) URL.revokeObjectURL(pendingRef.current.previewUrl);
    pendingRef.current = null;
    const recorder = recorderRef.current;
    if (recorder?.state === "recording") {
      recorder.ondataavailable = null;
      recorder.onerror = null;
      recorder.onstop = null;
      recorder.stop();
    }
    recorderRef.current = null;
    stopTracks();
  }, []);

  const replacePending = (next: PendingReference | null) => {
    const previous = pendingRef.current;
    if (previous && previous.previewUrl !== next?.previewUrl) {
      URL.revokeObjectURL(previous.previewUrl);
    }
    pendingRef.current = next;
    setPending(next);
  };

  const importFile = async (file: File | undefined) => {
    if (!file) return;
    const operation = ++operationRef.current;
    setError(null);
    setExcerptNotice(null);
    if (!file.size) {
      setError("The selected voice recording is empty.");
      return;
    }
    if (file.size > 32 * 1024 * 1024) {
      setError("Choose a voice reference smaller than 32 MiB.");
      return;
    }
    try {
      const durationSeconds = await audioDuration(file);
      if (operation !== operationRef.current) return;
      replacePending({
        blob: file,
        source: "imported",
        fileName: file.name,
        durationSeconds,
        previewUrl: URL.createObjectURL(file),
      });
      if (!name.trim()) setName(file.name.replace(/\.[^.]+$/, "").replace(/[-_]+/g, " "));
    } catch (cause) {
      if (operation === operationRef.current) setError(formatErrorMessage(cause));
    }
  };

  const handleCreateExcerpt = async (startSeconds?: number) => {
    if (
      !pending
      || pending.durationSeconds <= IDEAL_MAX_SECONDS
      || pending.durationSeconds > MAX_EXCERPT_ANALYSIS_SECONDS
    ) return;
    const operation = ++operationRef.current;
    setFindingExcerpt(true);
    setError(null);
    setExcerptNotice(null);
    try {
      const result = await createVoiceReferenceExcerpt(pending.blob, {
        knownDurationSeconds: pending.durationSeconds,
        startSeconds,
      });
      if (operation !== operationRef.current) return;
      replacePending({
        blob: result.blob,
        source: pending.source,
        fileName: `${pending.fileName.replace(/(?:-excerpt)?\.[^.]+$/i, "")}-excerpt.wav`,
        durationSeconds: result.durationSeconds,
        previewUrl: URL.createObjectURL(result.blob),
      });
      setExcerptNotice(
        `Created one continuous ${result.durationSeconds.toFixed(1)}-second excerpt from ${formatTimestamp(result.startSeconds)}–${formatTimestamp(result.endSeconds)}. Listen once before saving; activity analysis cannot identify the speaker.`,
      );
    } catch (cause) {
      if (operation === operationRef.current) {
        const message = formatErrorMessage(cause);
        const guidance = message.includes("could not decode the recording")
          ? ` ${diagnoseAudioDecodeError(pending.blob)}`
          : "";
        setError(`Could not create a voice excerpt: ${message}${guidance}`);
      }
    } finally {
      if (operation === operationRef.current) setFindingExcerpt(false);
    }
  };

  const stopRecording = () => {
    const recorder = recorderRef.current;
    if (recorder?.state === "recording") recorder.stop();
  };

  const startRecording = async () => {
    const operation = ++operationRef.current;
    setError(null);
    setExcerptNotice(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      });
      if (operation !== operationRef.current) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      streamRef.current = stream;
      const preferred = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus"]
        .find((type) => MediaRecorder.isTypeSupported(type));
      const recorder = new MediaRecorder(stream, preferred ? { mimeType: preferred } : undefined);
      recorderRef.current = recorder;
      chunksRef.current = [];
      startedRef.current = performanceNow();
      recorder.ondataavailable = (event) => {
        if (event.data.size) chunksRef.current.push(event.data);
      };
      recorder.onerror = () => {
        setError("Microphone recording failed. Check Windows microphone permission and try again.");
        setRecording(false);
        stopTracks();
      };
      recorder.onstop = () => {
        if (recorderRef.current === recorder) recorderRef.current = null;
        const durationSeconds = Math.max(0, (performanceNow() - startedRef.current) / 1000);
        const blob = new Blob(chunksRef.current, { type: recorder.mimeType || "audio/webm" });
        chunksRef.current = [];
        setRecording(false);
        stopTracks();
        if (!blob.size) {
          setError("The microphone did not return any audio.");
          return;
        }
        replacePending({
          blob,
          source: "recorded",
          fileName: `recorded-voice-${new Date().toISOString().replace(/[:.]/g, "-")}.webm`,
          durationSeconds,
          previewUrl: URL.createObjectURL(blob),
        });
        if (!name.trim()) setName("My voice");
      };
      recorder.start(250);
      setRecordingSeconds(0);
      setRecording(true);
      timerRef.current = window.setInterval(() => {
        const seconds = (performanceNow() - startedRef.current) / 1000;
        setRecordingSeconds(seconds);
        if (seconds >= MAX_REFERENCE_SECONDS) stopRecording();
      }, 100);
    } catch (cause) {
      if (operation === operationRef.current) {
        stopTracks();
        setError(`Microphone unavailable: ${formatErrorMessage(cause)}`);
      }
    }
  };

  const saveVoice = async () => {
    if (!pending) return;
    setBusy("create");
    setError(null);
    try {
      const request: CreateVoiceProfileRequest = {
        name,
        language,
        tags: tags.split(",").map((tag) => tag.trim()).filter(Boolean),
        source: pending.source,
        consentConfirmed: consent,
        performance,
        audioBase64: await blobAsBase64(pending.blob),
        mimeType: pending.blob.type || "application/octet-stream",
        originalFileName: pending.fileName,
        durationSeconds: pending.durationSeconds,
      };
      const next = await createVoiceProfile(request);
      onSnapshot(next);
      replacePending(null);
      setExcerptNotice(null);
      setAdding(false);
      setName("");
      setTags("");
      setConsent(false);
    } catch (cause) {
      setError(formatErrorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const makeDefault = async (profile: VoiceProfile) => {
    setBusy(profile.id);
    setError(null);
    try {
      onSnapshot(await setDefaultVoiceProfile(profile.id));
    } catch (cause) {
      setError(formatErrorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const changePerformance = async (profile: VoiceProfile, nextPerformance: VoicePerformance) => {
    if (profile.source === "built-in") return;
    setBusy(profile.id);
    setError(null);
    try {
      onSnapshot(await updateVoiceProfile({
        id: profile.id,
        name: profile.name,
        language: profile.language,
        tags: profile.tags,
        consentConfirmed: profile.consentConfirmed,
        performance: nextPerformance,
      }));
    } catch (cause) {
      setError(formatErrorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const removeVoice = async (profile: VoiceProfile) => {
    if (!window.confirm(`Delete “${profile.name}” from this private Voice Library? Existing generated speech remains preserved.`)) return;
    setBusy(profile.id);
    setError(null);
    try {
      onSnapshot(await deleteVoiceProfile(profile.id));
    } catch (cause) {
      setError(formatErrorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const assessment = pending ? assessVoiceReference(pending.durationSeconds) : null;
  const interactionBusy = !!busy || findingExcerpt;

  return (
    <dialog
      ref={dialogRef}
      className="voice-library-dialog"
      aria-labelledby="voice-library-title"
      onCancel={(event) => { event.preventDefault(); if (!recording && !busy) onClose(); }}
      onClose={onClose}
    >
      <header>
        <span className="voice-library-symbol"><Library /></span>
        <div><span className="eyebrow">Private and offline</span><h2 id="voice-library-title">Voice Library</h2><p>Cast one voice across Kestrel or create a voice from a recording you have permission to use.</p></div>
        <button type="button" aria-label="Close Voice Library" disabled={recording || !!busy} onClick={onClose}><X /></button>
      </header>

      <div className="voice-library-body">
        <section className="voice-profile-grid" aria-label="Available voices">
          {snapshot.profiles.map((profile) => {
            const selected = snapshot.defaultProfileId === profile.id;
            return (
              <article className={`voice-profile-card ${selected ? "selected" : ""}`} key={profile.id}>
                <div className="voice-profile-heading">
                  <span className="voice-avatar">{profile.name.slice(0, 1).toUpperCase()}</span>
                  <div><strong>{profile.name}</strong><small>{profile.source === "built-in" ? "Included neutral voice" : `${profile.language} · ${profile.referenceSeconds?.toFixed(1)} seconds`}</small></div>
                  {selected && <span className="voice-default-badge"><Check /> Default</span>}
                </div>
                <div className="voice-tags">{profile.tags.map((tag) => <span key={tag}>{tag}</span>)}</div>
                {profile.referenceRelativePath && <audio controls preload="metadata" src={localSpeechMediaUrl(profile.referenceRelativePath)} aria-label={`${profile.name} reference recording`} />}
                <label>Performance
                  <select
                    aria-label={`${profile.name} performance`}
                    value={profile.performance}
                    disabled={profile.source === "built-in" || interactionBusy}
                    onChange={(event) => void changePerformance(profile, event.target.value as VoicePerformance)}
                  >
                    <option value="restrained">Restrained</option>
                    <option value="natural">Natural</option>
                    <option value="expressive">Expressive</option>
                    <option value="dramatic">Dramatic</option>
                  </select>
                </label>
                <footer>
                  <button type="button" disabled={selected || interactionBusy} onClick={() => void makeDefault(profile)}>{selected ? <Check /> : <Play />} {selected ? "Current default" : "Use across Kestrel"}</button>
                  {profile.source !== "built-in" && <button type="button" className="danger" aria-label={`Delete ${profile.name}`} disabled={interactionBusy} onClick={() => void removeVoice(profile)}><Trash2 /></button>}
                </footer>
              </article>
            );
          })}
        </section>

        {!adding ? (
          <button type="button" className="voice-add-button" disabled={interactionBusy} onClick={() => setAdding(true)}><Plus /> Add a custom voice</button>
        ) : (
          <section className="voice-create-panel" aria-labelledby="voice-create-title">
            <div><span className="eyebrow">New custom voice</span><h3 id="voice-create-title">Record or import one clean speaker</h3><p>Speak naturally without music or another voice. Kestrel saves only the reference you review and uses it only on this PC.</p></div>
            <div className="voice-source-actions">
              <button type="button" className={recording ? "recording" : ""} disabled={interactionBusy} onClick={() => recording ? stopRecording() : void startRecording()}>{recording ? <Pause /> : <Mic />} {recording ? `Stop · ${recordingSeconds.toFixed(1)}s` : "Record voice"}</button>
              <label className={`button-like ${recording || interactionBusy ? "disabled" : ""}`} aria-disabled={recording || interactionBusy}><FolderOpen /> Import audio<input type="file" accept="audio/wav,audio/flac,audio/mpeg,audio/ogg,audio/webm,audio/mp4,.wav,.flac,.mp3,.ogg,.opus,.webm,.m4a" disabled={recording || interactionBusy} onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; void importFile(file); }} /></label>
            </div>
            <p className="voice-source-help">WAV, MP3, FLAC, Ogg/Opus, WebM, or M4A · 3–45 seconds · Max 32 MiB</p>
            {pending && (
              <div className="voice-reference-review">
                <audio ref={pendingAudioRef} controls src={pending.previewUrl} />
                <div className="voice-reference-info">
                  <span className={assessment?.tone}>{assessment?.text}</span>
                  {pending.durationSeconds > IDEAL_MAX_SECONDS && pending.durationSeconds <= MAX_EXCERPT_ANALYSIS_SECONDS && (
                    <div className="voice-excerpt-action">
                      <button
                        type="button"
                        className="voice-excerpt-button"
                        disabled={interactionBusy}
                        title="Use one continuous passage beginning at the player's current position"
                        onClick={() => void handleCreateExcerpt(pendingAudioRef.current?.currentTime ?? 0)}
                      >
                        <Scissors /> {findingExcerpt ? "Creating excerpt…" : `Use ${VOICE_EXCERPT_SECONDS}s from playhead`}
                      </button>
                      <button
                        type="button"
                        className="voice-excerpt-button"
                        disabled={interactionBusy}
                        title="Find one continuous high-activity passage; this does not identify speakers"
                        onClick={() => void handleCreateExcerpt()}
                      >
                        <Wand2 /> {findingExcerpt ? "Creating excerpt…" : `Find active ${VOICE_EXCERPT_SECONDS}s`}
                      </button>
                      <small>Scrub the player to choose a start, or let Kestrel find a high-activity passage. Listen for music or another speaker before saving.</small>
                    </div>
                  )}
                  {excerptNotice && <small className="voice-excerpt-notice" role="status">{excerptNotice}</small>}
                </div>
              </div>
            )}
            <div className="voice-fields">
              <label>Voice name<input value={name} maxLength={80} disabled={interactionBusy} onChange={(event) => setName(event.target.value)} placeholder="Evening narrator" /></label>
              <label>Language or accent<input value={language} maxLength={40} disabled={interactionBusy} onChange={(event) => setLanguage(event.target.value)} placeholder="Auto" /></label>
              <label>Character tags<input value={tags} disabled={interactionBusy} onChange={(event) => setTags(event.target.value)} placeholder="warm, mature, documentary" /></label>
              <label>Default performance<select value={performance} disabled={interactionBusy} onChange={(event) => setPerformance(event.target.value as VoicePerformance)}><option value="restrained">Restrained</option><option value="natural">Natural</option><option value="expressive">Expressive</option><option value="dramatic">Dramatic</option></select></label>
            </div>
            <label className="voice-consent"><input type="checkbox" checked={consent} disabled={interactionBusy} onChange={(event) => setConsent(event.target.checked)} /> I own this recording or have permission to create and use this voice.</label>
            <footer>
              <button type="button" disabled={recording || !!busy} onClick={() => { operationRef.current++; setFindingExcerpt(false); replacePending(null); setExcerptNotice(null); setAdding(false); }}>Cancel</button>
              <button
                type="button"
                className="primary-button"
                disabled={!pending || assessment?.tone === "bad" || !name.trim() || !consent || interactionBusy}
                title={
                  !pending
                    ? "Record or import a voice reference first"
                    : assessment?.tone === "bad"
                      ? assessment.text
                      : !name.trim()
                        ? "Enter a voice name"
                        : !consent
                          ? "Confirm permission checkbox"
                          : undefined
                }
                onClick={() => void saveVoice()}
              >
                <Check /> {busy === "create" ? "Saving locally…" : "Save voice"}
              </button>
            </footer>
          </section>
        )}
        {error && <p className="voice-library-error" role="alert">{error}</p>}
      </div>
    </dialog>
  );
}

function performanceNow(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function formatTimestamp(seconds: number): string {
  const bounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(bounded / 60);
  const remainder = bounded % 60;
  return `${minutes}:${remainder.toString().padStart(2, "0")}`;
}
