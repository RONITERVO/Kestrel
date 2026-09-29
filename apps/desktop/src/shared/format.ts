/** GPU and model memory in the units NVIDIA tools show: MiB below 1 GiB, GiB above. */
export function formatMib(value: number | undefined | null): string {
  // Zero is a real reading (an exhausted GPU has 0 MiB free); only a missing value is unknown.
  if (value == null || !Number.isFinite(value)) return "—";
  return value >= 1024 ? `${(value / 1024).toFixed(1)} GiB` : `${value.toLocaleString()} MiB`;
}

/** What an export saved, for its status line: the audio's name, what came with it, and where. */
export function describeExportedFiles(files: string[]): string {
  const [audio, ...companions] = files;
  if (!audio) return "Nothing was saved.";
  const cut = Math.max(audio.lastIndexOf("\\"), audio.lastIndexOf("/"));
  const folder = cut > 0 ? audio.slice(0, cut) : "";
  const name = audio.slice(cut + 1);
  const extras = companions.length ? " with its word timings (LRC, WebVTT, JSON) and a word-by-word player page" : "";
  return `Saved ${name}${extras}${folder ? ` in ${folder}` : ""}.`;
}

export function formatDate(value: string): string {
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" }).format(new Date(value));
}
