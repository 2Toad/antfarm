// Formatting that matches antfarm-view's Rust helpers, so the app and the terminal read
// the same.

/** `45s`, `4m12s`, `1h03m`, `2d4h`. */
export function dur(secs: number | null | undefined): string {
  if (secs == null) return "-";
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
  if (s < 86400) return `${Math.floor(s / 3600)}h${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
  return `${Math.floor(s / 86400)}d${Math.floor((s % 86400) / 3600)}h`;
}

/** Milliseconds as `0.4s`, `12.4s`, or a duration. */
export function ms(v: number | null | undefined): string {
  if (v == null) return "-";
  if (v < 60_000) return `${(v / 1000).toFixed(1)}s`;
  return dur(v / 1000);
}

/** `HH:MM:SS` in local time. */
export function clock(iso: string | null | undefined): string {
  if (!iso) return "-";
  const d = new Date(iso);
  return d.toLocaleTimeString([], { hour12: false });
}

export function clockHM(iso: string | null | undefined): string {
  if (!iso) return "-";
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
}

export function rate(v: number): string {
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (v >= 10_000) return `${Math.round(v / 1000)}k`;
  if (v >= 1000) return `${(v / 1000).toFixed(1)}k`;
  return `${Math.round(v)}`;
}

export function pct(v: number | null | undefined): string {
  return v == null ? "-" : `${Math.round(v)}%`;
}

export function secsSince(iso: string | null | undefined, now: string): number | null {
  if (!iso) return null;
  return (new Date(now).getTime() - new Date(iso).getTime()) / 1000;
}

/** The last path segment, for long paths in tight columns. */
export function base(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

/** `~/…` for paths under the home directory, when it can be guessed. */
export function tilde(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
}
