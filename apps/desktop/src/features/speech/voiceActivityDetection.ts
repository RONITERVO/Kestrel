/**
 * Voice Activity Detection (VAD) for automatic microphone dictation shutoff.
 * Uses the Web Audio API AnalyserNode to calculate real-time RMS energy and
 * cleanly trigger an auto-stop after user-configured silence.
 */

export interface VadSettings {
  /** Whether VAD auto-stop is enabled */
  enabled: boolean;
  /** Silence duration in seconds required to auto-stop recording after speech has started */
  silenceTimeoutSec: number;
  /** Audio energy threshold in dB to classify a frame as speech vs silence (-60 dB sensitive to -20 dB noisy) */
  speechThresholdDb: number;
  /** Minimum continuous speech duration in milliseconds before silence auto-stop is armed */
  minSpeechDurationMs: number;
  /** Maximum initial silence in seconds before auto-stopping if the user never begins speaking */
  initialGraceTimeoutSec: number;
}

export const DEFAULT_VAD_SETTINGS: VadSettings = {
  enabled: true,
  silenceTimeoutSec: 2.0,
  speechThresholdDb: -42,
  minSpeechDurationMs: 400,
  initialGraceTimeoutSec: 15.0,
};

const STORAGE_KEY = "kestrel_speech_vad_settings";

export function normalizeVadSettings(raw?: Partial<VadSettings> | null): VadSettings {
  if (!raw || typeof raw !== "object") {
    return { ...DEFAULT_VAD_SETTINGS };
  }

  const enabled = typeof raw.enabled === "boolean" ? raw.enabled : DEFAULT_VAD_SETTINGS.enabled;

  const silenceTimeoutSec =
    typeof raw.silenceTimeoutSec === "number" && Number.isFinite(raw.silenceTimeoutSec)
      ? Math.max(0.5, Math.min(10.0, raw.silenceTimeoutSec))
      : DEFAULT_VAD_SETTINGS.silenceTimeoutSec;

  const speechThresholdDb =
    typeof raw.speechThresholdDb === "number" && Number.isFinite(raw.speechThresholdDb)
      ? Math.max(-60, Math.min(-20, Math.round(raw.speechThresholdDb)))
      : DEFAULT_VAD_SETTINGS.speechThresholdDb;

  const minSpeechDurationMs =
    typeof raw.minSpeechDurationMs === "number" && Number.isFinite(raw.minSpeechDurationMs)
      ? Math.max(100, Math.min(2000, Math.round(raw.minSpeechDurationMs)))
      : DEFAULT_VAD_SETTINGS.minSpeechDurationMs;

  const initialGraceTimeoutSec =
    typeof raw.initialGraceTimeoutSec === "number" && Number.isFinite(raw.initialGraceTimeoutSec)
      ? Math.max(3.0, Math.min(60.0, raw.initialGraceTimeoutSec))
      : DEFAULT_VAD_SETTINGS.initialGraceTimeoutSec;

  return {
    enabled,
    silenceTimeoutSec,
    speechThresholdDb,
    minSpeechDurationMs,
    initialGraceTimeoutSec,
  };
}

export function loadVadSettings(): VadSettings {
  if (typeof window === "undefined" || !window.localStorage) {
    return { ...DEFAULT_VAD_SETTINGS };
  }
  try {
    const serialized = window.localStorage.getItem(STORAGE_KEY);
    if (!serialized) return { ...DEFAULT_VAD_SETTINGS };
    const parsed = JSON.parse(serialized) as unknown;
    return normalizeVadSettings(parsed as Partial<VadSettings>);
  } catch {
    return { ...DEFAULT_VAD_SETTINGS };
  }
}

export function saveVadSettings(settings: VadSettings): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    const normalized = normalizeVadSettings(settings);
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(normalized));
  } catch {
    // Local storage quota or security restriction fallback
  }
}

export interface VadCallbacks {
  onSilenceTimeout: () => void;
  onEnergyUpdate?: (db: number, isSpeaking: boolean, silenceProgressRatio: number) => void;
  onSpeechStart?: () => void;
}

export class VoiceActivityDetector {
  private audioContext: AudioContext | null = null;
  private analyser: AnalyserNode | null = null;
  private sourceNode: MediaStreamAudioSourceNode | null = null;
  private timer: number | null = null;
  private dataArray: Float32Array | null = null;

  private hasSpoken = false;
  private consecutiveSpeechMs = 0;
  private consecutiveSilenceMs = 0;
  private initialSilenceMs = 0;
  private destroyed = false;
  private readonly sampleIntervalMs = 50;
  private previousSampleTimestampMs = 0;

  constructor(
    private readonly stream: MediaStream,
    private readonly settings: VadSettings,
    private readonly callbacks: VadCallbacks,
  ) {
    if (!settings.enabled) return;
    this.initAudioPipeline();
  }

  private initAudioPipeline() {
    try {
      const AudioCtx = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      if (!AudioCtx) return;

      const ctx = new AudioCtx();
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      analyser.smoothingTimeConstant = 0.2;

      const source = ctx.createMediaStreamSource(this.stream);
      source.connect(analyser);

      this.audioContext = ctx;
      this.analyser = analyser;
      this.sourceNode = source;
      this.dataArray = new Float32Array(analyser.fftSize);

      if (ctx.state === "suspended") {
        void ctx.resume();
      }

      this.startLoop();
    } catch {
      // If Web Audio fails, graceful fallback to manual stop
      this.destroy();
    }
  }

  private startLoop() {
    if (this.destroyed || this.timer !== null) return;
    this.previousSampleTimestampMs = performance.now();

    this.timer = window.setInterval(() => {
      if (this.destroyed || !this.analyser || !this.dataArray) return;
      const now = performance.now();
      const deltaMs = Math.max(0, now - this.previousSampleTimestampMs);
      this.previousSampleTimestampMs = now;

      this.analyser.getFloatTimeDomainData(this.dataArray);

      // Compute RMS (Root Mean Square) energy
      let sumSquares = 0;
      for (let i = 0; i < this.dataArray.length; i++) {
        const val = this.dataArray[i];
        sumSquares += val * val;
      }
      const rms = Math.sqrt(sumSquares / this.dataArray.length);
      const db = Math.max(-100, Math.min(0, 20 * Math.log10(rms + 1e-7)));

      const isSpeechFrame = db >= this.settings.speechThresholdDb;

      if (isSpeechFrame) {
        this.consecutiveSpeechMs += deltaMs;
        this.consecutiveSilenceMs = 0;

        if (this.consecutiveSpeechMs >= this.settings.minSpeechDurationMs) {
          if (!this.hasSpoken) this.callbacks.onSpeechStart?.();
          this.hasSpoken = true;
        }

        this.callbacks.onEnergyUpdate?.(db, true, 0);
      } else {
        this.consecutiveSpeechMs = 0;

        if (this.hasSpoken) {
          this.consecutiveSilenceMs += deltaMs;
          const targetSilenceMs = this.settings.silenceTimeoutSec * 1000;
          const silenceRatio = Math.min(1, this.consecutiveSilenceMs / targetSilenceMs);

          this.callbacks.onEnergyUpdate?.(db, false, silenceRatio);

          if (this.consecutiveSilenceMs >= targetSilenceMs) {
            this.destroy();
            this.callbacks.onSilenceTimeout();
            return;
          }
        } else {
          this.initialSilenceMs += deltaMs;
          const targetInitialGraceMs = this.settings.initialGraceTimeoutSec * 1000;
          const silenceRatio = Math.min(1, this.initialSilenceMs / targetInitialGraceMs);

          this.callbacks.onEnergyUpdate?.(db, false, silenceRatio);

          if (this.initialSilenceMs >= targetInitialGraceMs) {
            this.destroy();
            this.callbacks.onSilenceTimeout();
            return;
          }
        }
      }
    }, this.sampleIntervalMs);
  }

  public destroy() {
    this.destroyed = true;
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
    if (this.sourceNode) {
      try {
        this.sourceNode.disconnect();
      } catch {
        // Ignored
      }
      this.sourceNode = null;
    }
    if (this.analyser) {
      try {
        this.analyser.disconnect();
      } catch {
        // Ignored
      }
      this.analyser = null;
    }
    if (this.audioContext && this.audioContext.state !== "closed") {
      try {
        void this.audioContext.close();
      } catch {
        // Ignored
      }
      this.audioContext = null;
    }
  }
}
