// The banner for a file another program changed: what each tab was told, and when the banner shows.

import type { DiskEvent } from "./backend";

/** How often the open files are looked at while the window has the focus. */
export const DISK_EVERY = 20_000;
/** A tab switch looks again only after this long. */
export const DISK_AGAIN = 3_000;

export interface DiskNote {
  state: "changed" | "gone";
  /** Changes when the file changes again. */
  token: string;
  /** When the other program wrote it (ms since 1970). */
  modified: number | null;
}

/** What a tab keeps about its file on disk. */
export interface DiskTab {
  diskNote: DiskNote | null;
  /** The change the person chose to ignore. */
  diskIgnored: string;
  /** "Don't warn for this file" (until the tab closes). */
  diskMuted: boolean;
}

/** Take in one answer about a tab's file. */
export function takeDisk(t: DiskTab, e: DiskEvent): void {
  if (e.state === "same") {
    t.diskNote = null;
    t.diskIgnored = "";
    return;
  }
  const same = t.diskNote?.state === e.state && t.diskNote.token === e.token;
  if (!same) t.diskNote = { state: e.state, token: e.token, modified: e.modified };
}

/** The banner to show for a tab, if any. */
export function diskBanner(t: DiskTab): DiskNote | null {
  const n = t.diskNote;
  return !n || t.diskMuted || t.diskIgnored === n.token ? null : n;
}

export function diskText(name: string, n: DiskNote, time: (ms: number) => string = clock): string {
  if (n.state === "gone") return `${name} was deleted or moved. Keep working, then use Save As.`;
  const at = n.modified === null ? "" : ` at ${time(n.modified)}`;
  return `${name} changed on disk (another program saved it${at}).`;
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
