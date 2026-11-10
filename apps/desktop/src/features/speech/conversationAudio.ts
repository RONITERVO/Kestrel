/** Bounded, disposable audio presentation state. Durable turns still go through native IPC. */
export function pcmBase64(samples: Float32Array): string {
  const bytes = new Uint8Array(samples.length * 2);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < samples.length; i++) view.setInt16(i * 2, Math.round(Math.max(-1, Math.min(1, samples[i])) * 32767), true);
  let binary = "";
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(binary);
}

export function decodePcm(base64: string): Float32Array {
  const binary = atob(base64);
  if (binary.length % 2 || binary.length > 4800) throw new Error("Invalid conversation audio frame");
  const bytes = Uint8Array.from(binary, (v) => v.charCodeAt(0));
  const view = new DataView(bytes.buffer);
  return Float32Array.from({ length: bytes.length / 2 }, (_, i) => view.getInt16(i * 2, true) / 32768);
}

export class UtteranceGate {
  private prefix: Float32Array[] = [];
  private frames: Float32Array[] = [];
  private voiceMs = 0;
  private silenceMs = 0;
  private samples = 0;
  private speaking = false;
  constructor(private readonly onStart: () => void, private readonly onUtterance: (pcm: Float32Array) => void,
    private readonly threshold = -42, private readonly silence = 600, private readonly minimumSpeech = 200) {}

  push(frame: Float32Array) {
    const ms = frame.length / 16;
    const power = frame.reduce((sum, sample) => sum + sample * sample, 0) / frame.length;
    const speech = 10 * Math.log10(power + 1e-10) >= this.threshold;
    if (!this.frames.length) {
      this.prefix.push(frame);
      if (this.prefix.length > 10) this.prefix.shift();
      if (speech) { this.frames = this.prefix.splice(0); this.samples = this.frames.reduce((n, f) => n + f.length, 0); }
      else return;
    } else { this.frames.push(frame); this.samples += frame.length; }
    if (speech) {
      this.voiceMs += ms;
      this.silenceMs = 0;
      if (!this.speaking && this.voiceMs >= this.minimumSpeech) { this.speaking = true; this.onStart(); }
    } else {
      this.silenceMs += ms;
      if (!this.speaking) this.voiceMs = 0;
    }
    if (this.silenceMs >= this.silence || this.samples >= 480000) {
      if (this.speaking) {
        const pcm = new Float32Array(this.samples);
        let at = 0;
        for (const part of this.frames) { pcm.set(part, at); at += part.length; }
        this.onUtterance(pcm);
      }
      this.reset();
    }
  }

  reset() { this.prefix = []; this.frames = []; this.voiceMs = 0; this.silenceMs = 0; this.samples = 0; this.speaking = false; }
}

/** Stream clauses without reading fenced code, reasoning tags, or unfinished markup aloud. */
export class SpokenClauses {
  private pending = "";
  private hidden: "code" | "think" | null = null;
  append(text: string, final = false): string[] {
    this.pending += text;
    if (this.pending.length > 32768) throw new Error("The spoken reply queue exceeded its limit. The written reply remains available.");
    const result: string[] = [];
    while (this.pending) {
      if (this.hidden) {
        const close = this.hidden === "code" ? "```" : "</think>";
        const end = this.pending.indexOf(close);
        if (end < 0) { this.pending = this.pending.slice(-(close.length - 1)); break; }
        this.pending = this.pending.slice(end + close.length); this.hidden = null; continue;
      }
      const marker = /```|<think>/.exec(this.pending);
      const boundary = /[.!?;]\s|\n/.exec(this.pending);
      let end = boundary ? boundary.index + 1 : -1;
      if (marker && marker.index <= 180 && (end < 0 || marker.index <= end)) {
        const before = this.pending.slice(0, marker.index);
        const spoken = cleanSpeech(before);
        if (spoken) result.push(spoken);
        this.pending = this.pending.slice(marker.index + marker[0].length);
        this.hidden = marker[0] === "```" ? "code" : "think";
        continue;
      }
      if ((end < 0 || end > 180) && this.pending.length >= 180) end = this.pending.lastIndexOf(" ", 180) + 1;
      if (end <= 0 && !final) break;
      if (end <= 0) end = this.pending.length;
      // A possibly split opening tag/fence is withheld until it can be classified.
      const part = this.pending.slice(0, end);
      if (!final && /(?:<[^>]*|`{1,2})$/.test(part)) break;
      const spoken = cleanSpeech(part);
      if (spoken) result.push(spoken);
      this.pending = this.pending.slice(end);
    }
    return result;
  }
}

function cleanSpeech(text: string): string {
  return text.replace(/!\[[^\]]*\]\([^)]*\)/g, "").replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/`[^`]*`/g, "").replace(/<[^>]*>/g, "").replace(/https?:\/\/\S+/g, "")
    .replace(/[*_#>|]/g, "").replace(/\s+/g, " ").trim();
}
