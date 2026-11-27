// Tab colour: a line under a tab (a border, so Windows high contrast keeps it) and its name for screen readers.
// Kept with the remembered session ("Pick up where you left off"), which never holds a private tab; named views
// (View › Save view) leave it out, as they are for any file with the same titles.

import type { View } from "./views";

export const TAB_COLOURS: readonly { name: string; css: string }[] = [
  { name: "red", css: "#d64545" },
  { name: "orange", css: "#e07b1f" },
  { name: "yellow", css: "#c9a400" },
  { name: "green", css: "#2f9e44" },
  { name: "teal", css: "#0c8599" },
  { name: "blue", css: "#3b6fd8" },
  { name: "purple", css: "#8b4fd1" },
  { name: "grey", css: "#7a828c" },
];

/** A colour number from a view or the UI (untrusted), or null. */
export function cleanColour(raw: unknown): number | null {
  return typeof raw === "number" && Number.isInteger(raw) && raw >= 0 && raw < TAB_COLOURS.length ? raw : null;
}

/** What a screen reader hears for a tab. */
export function tabLabel(name: string, colour: number | null): string | undefined {
  const c = colour === null ? undefined : TAB_COLOURS[colour];
  return c ? `${name}, tab colour ${c.name}` : undefined;
}

/** Tabs the remembered session keeps: a file on disk that is not private. A private tab (and so its colour) is
 *  never written there. */
export function rememberable(t: { info: { path: string | null }; private: { file: boolean; folder: boolean } }): boolean {
  return !!t.info.path && !t.private.file && !t.private.folder;
}

/** A named view leaves the tab colour out (it belongs to the tab, not to files with the same titles). */
export function namedView(v: View): View {
  const { tab_colour: _drop, ...rest } = v;
  return rest;
}

/** "v.csv (copy)", then "v.csv (copy 2)", "v.csv (copy 3)": the first name no open tab has (case ignored). */
export function copyName(name: string, taken: readonly string[]): string {
  const base = name.replace(/ \(copy(?: \d+)?\)$/, "");
  const used = new Set(taken.map((x) => x.toLowerCase()));
  for (let n = 1; ; n++) {
    const c = n === 1 ? `${base} (copy)` : `${base} (copy ${n})`;
    if (!used.has(c.toLowerCase())) return c;
  }
}
