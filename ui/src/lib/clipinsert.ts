// Insert copied rows or columns (Excel's Ctrl+Shift++ after a copy): what was copied goes in as new rows above
// the selection, or new columns before it, instead of over the cells there. The values are always the clipboard
// text (what was copied, shown rows only, as Paste reads it); widths and formats come from the source only while
// it is unchanged since the copy.

/** What the last Copy of whole rows or columns was (null after any other copy or cut). */
export interface CopyRec {
  /** The tab it was copied from. */
  from: number;
  kind: "rows" | "cols";
  rect: { row: number; rows: number; col: number; cols: number };
  /** The text put on the clipboard. */
  clip: string;
  /** The source's history position at the copy: its widths and formats are read only while it is the same. */
  pos: number;
  /** The source had a title row. */
  header: boolean;
  /** The source was private or encrypted at the copy. */
  shut: boolean;
}

/** The insert-copied key: Ctrl+Shift++ on the main keyboard (code Equal with Shift, key "+") or on the number pad,
 *  and Ctrl+NumpadAdd. Never with Alt or AltGr (Ctrl+Alt is AltGr on Windows, which types characters). */
export function isInsertCopiedKey(e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey"> & { getModifierState?: (k: string) => boolean }): boolean {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.getModifierState?.("AltGraph")) return false;
  if (e.code === "NumpadAdd") return true;
  return e.shiftKey && e.key === "+" && e.code === "Equal";
}

/** Clipboard text compared loosely (line ends, a trailing line break), as Paste compares a cut. */
export const sameClip = (s: string): string => s.replace(/\r\n?/g, "\n").replace(/\n+$/, "");

export type Offer = { ok: true; kind: "rows" | "cols"; count: number } | { ok: false; why: string };

/** Whether Insert copied may run now, and why not. `clipNow`: the clipboard text read now (null: not read yet, or it
 *  could not be read; the insert reads it again). */
export function insertOffer(rec: CopyRec | null, clipNow: string | null): Offer {
  if (!rec) return { ok: false, why: "Copy whole rows or whole columns first (Ctrl+C), then pick where they go" };
  if (clipNow !== null && sameClip(clipNow) !== sameClip(rec.clip)) return { ok: false, why: "The clipboard holds something from another program" };
  return { ok: true, kind: rec.kind, count: rec.kind === "rows" ? rec.rect.rows : rec.rect.cols };
}

/** The menu words for it. */
export function insertLabel(o: Offer): string {
  if (!o.ok) return "Insert copied rows or columns";
  const n = o.count.toLocaleString();
  return o.kind === "rows" ? `Insert ${o.count === 1 ? "1 copied row" : `${n} copied rows`} above` : `Insert ${o.count === 1 ? "1 copied column" : `${n} copied columns`} left`;
}
