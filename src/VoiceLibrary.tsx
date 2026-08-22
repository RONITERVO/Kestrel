import { Check, FolderOpen, Library, Mic, Pause, Play, Plus, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  createVoiceProfile,
  deleteVoiceProfile,
  localSpeechMediaUrl,
  setDefaultVoiceProfile,
  updateVoiceProfile,
} from "./api";
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
    const finish = () => URL.revokeObjectURL(url);
    audio.onloadedmetadata = () => {
      const duration = audio.duration;
      finish();
      if (!Number.isFinite(duration) || duration <= 0) {
        reject(new Error("Kestrel could not measure this recording."));
      } else {
        resolve(duration);
      }
    };
    audio.onerror = () => {
      finish();
      reject(new Error("This audio file could not be decoded."));
    };
    audio.preload = "metadata";
    audio.src = url;
  });
}

export function assessVoiceReference(duration: number): { tone: string; text: string } {
  if (duration < MIN_REFERENCE_SECONDS) {
    return { tone: "bad", text: `Record at least ${MIN_REFERENCE_SECONDS} seconds of clear speech.` };
  }
  if (duration > MAX_REFERENCE_SECONDS) {
    return { tone: "bad", text: `Trim the reference to ${MAX_REFERENCE_SECONDS} seconds or less.` };
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
  const [adding, setAdding] = useState(false);
  const [recording, setRecording] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const [pending, setPending] = useState<PendingReference | null>(null);
  const [name, setName] = useState("");
  const [language, setLanguage] = useState("Auto");
  const [tags, setTags] = useState("");
  const [performance, setPerformance] = useState<VoicePerformance>("natural");
  const [consent, setConsent] = useState(false);
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
    if (pending) URL.revokeObjectURL(pending.previewUrl);
    if (recorderRef.current?.state === "recording") recorderRef.current.stop();
    stopTracks();
  }, [pending]);

  const replacePending = (next: PendingReference | null) => {
    setPending((previous) => {
      if (previous) URL.revokeObjectURL(previous.previewUrl);
      return next;
    });
  };

  const importFile = async (file: File | undefined) => {
    if (!file) return;
    setError(null);
    if (file.size > 32 * 1024 * 1024) {
      setError("Choose a voice reference smaller than 32 MiB.");
      return;
    }
    try {
      const durationSeconds = await audioDuration(file);
      replacePending({
        blob: file,
        source: "imported",
        fileName: file.name,
        durationSeconds,
        previewUrl: URL.createObjectURL(file),
      });
      if (!name.trim()) setName(file.name.replace(/\.[^.]+$/, "").replace(/[-_]+/g, " "));
    } catch (cause) {
      setError(String(cause));
    }
  };

  const stopRecording = () => {
    const recorder = recorderRef.current;
    if (recorder?.state === "recording") recorder.stop();
  };

  const startRecording = async () => {
    setError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      });
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
      stopTracks();
      setError(`Microphone unavailable: ${String(cause)}`);
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
      setAdding(false);
      setName("");
      setTags("");
      setConsent(false);
    } catch (cause) {
      setError(String(cause));
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
      setError(String(cause));
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
      setError(String(cause));
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
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const assessment = pending ? assessVoiceReference(pending.durationSeconds) : null;

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
                    disabled={profile.source === "built-in" || !!busy}
                    onChange={(event) => void changePerformance(profile, event.target.value as VoicePerformance)}
                  >
                    <option value="restrained">Restrained</option>
                    <option value="natural">Natural</option>
                    <option value="expressive">Expressive</option>
                    <option value="dramatic">Dramatic</option>
                  </select>
                </label>
                <footer>
                  <button type="button" disabled={selected || !!busy} onClick={() => void makeDefault(profile)}>{selected ? <Check /> : <Play />} {selected ? "Current default" : "Use across Kestrel"}</button>
                  {profile.source !== "built-in" && <button type="button" className="danger" aria-label={`Delete ${profile.name}`} disabled={!!busy} onClick={() => void removeVoice(profile)}><Trash2 /></button>}
                </footer>
              </article>
            );
          })}
        </section>

        {!adding ? (
          <button type="button" className="voice-add-button" disabled={!!busy} onClick={() => setAdding(true)}><Plus /> Add a custom voice</button>
        ) : (
          <section className="voice-create-panel" aria-labelledby="voice-create-title">
            <div><span className="eyebrow">New custom voice</span><h3 id="voice-create-title">Record or import one clean speaker</h3><p>Speak naturally without music or another voice. Kestrel preserves this reference unchanged and uses it only on this PC.</p></div>
            <div className="voice-source-actions">
              <button type="button" className={recording ? "recording" : ""} disabled={!!busy} onClick={() => recording ? stopRecording() : void startRecording()}>{recording ? <Pause /> : <Mic />} {recording ? `Stop · ${recordingSeconds.toFixed(1)}s` : "Record voice"}</button>
              <label className="button-like"><FolderOpen /> Import audio<input type="file" accept="audio/wav,audio/flac,audio/mpeg,audio/ogg,audio/webm,audio/mp4,.wav,.flac,.mp3,.ogg,.opus,.webm,.m4a" disabled={recording || !!busy} onChange={(event) => void importFile(event.target.files?.[0])} /></label>
            </div>
            {pending && <div className="voice-reference-review"><audio controls src={pending.previewUrl} /><span className={assessment?.tone}>{pending.durationSeconds.toFixed(1)} seconds · {assessment?.text}</span></div>}
            <div className="voice-fields">
              <label>Voice name<input value={name} maxLength={80} disabled={!!busy} onChange={(event) => setName(event.target.value)} placeholder="Evening narrator" /></label>
              <label>Language or accent<input value={language} maxLength={40} disabled={!!busy} onChange={(event) => setLanguage(event.target.value)} placeholder="Auto" /></label>
              <label>Character tags<input value={tags} disabled={!!busy} onChange={(event) => setTags(event.target.value)} placeholder="warm, mature, documentary" /></label>
              <label>Default performance<select value={performance} disabled={!!busy} onChange={(event) => setPerformance(event.target.value as VoicePerformance)}><option value="restrained">Restrained</option><option value="natural">Natural</option><option value="expressive">Expressive</option><option value="dramatic">Dramatic</option></select></label>
            </div>
            <label className="voice-consent"><input type="checkbox" checked={consent} disabled={!!busy} onChange={(event) => setConsent(event.target.checked)} /> I own this recording or have permission to create and use this voice.</label>
            <footer><button type="button" disabled={recording || !!busy} onClick={() => { replacePending(null); setAdding(false); }}>Cancel</button><button type="button" className="primary-button" disabled={!pending || assessment?.tone === "bad" || !name.trim() || !consent || !!busy} onClick={() => void saveVoice()}><Check /> {busy === "create" ? "Saving locally…" : "Save voice"}</button></footer>
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
