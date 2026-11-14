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
