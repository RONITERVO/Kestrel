/** GPU and model memory in the units NVIDIA tools show: MiB below 1 GiB, GiB above. */
export function formatMib(value: number | undefined | null): string {
  if (!value) return "—";
  return value >= 1024 ? `${(value / 1024).toFixed(1)} GiB` : `${value.toLocaleString()} MiB`;
}

export function formatDate(value: string): string {
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" }).format(new Date(value));
}
