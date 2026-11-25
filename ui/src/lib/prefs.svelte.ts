// User preferences, kept in this machine's local storage. Reading or writing
// storage can fail (private mode, cleared data); defaults are used then.

import { MAX_VIEWS, safeFont, type Shortcuts, type View } from "./views";
import { SECOND_WINDOW } from "./windowRole";

export type Theme = "system" | "light" | "dark";

export interface PrefsData {
  theme: Theme;
  accent: string;
  font: string;
  fontSize: number;
  zoom: number;
  editPanel: boolean;
  gridLines: boolean;
  /** View › Show invisible characters (display only). */
  showInvisible: boolean;
  /** How a cell that breaks a rule shows: a red fill, a wavy underline, or both. */
  problemStyle: "fill" | "underline" | "both";
  /** Numbers and dates line up on the right (as in Excel), unless a column
   *  has its own alignment. */
  alignNumbers: boolean;
  /** Column headings also show spreadsheet letters (A, B, C). */
  colLetters: boolean;
  /** A strip under each heading: filled share, different values, a tiny chart. */
  insights: boolean;
  /** Where Enter moves after typing (Shift+Enter goes the other way). */
  enterMove: EnterMove;
  /** Columns with a list of allowed values show the list arrow on every
   *  cell (as in Google Sheets), not only on the active one. */
  listArrows: boolean;
  /** Underline misspelt words while editing cells. */
  spellcheck: boolean;
  /** The agent link was switched on (it starts again with the app). */
  agentLink: boolean;
  /** Custom keys by command id (cleaned against the commands at start). */
  shortcuts: Shortcuts;
  /** Saved views, by name. */
  views: View[];
  /** The side panel is open, and on which tab. */
  panel: "summary" | "chart" | "files" | "activity" | "filters" | "steps" | null;
  /** Which quick total the status bar shows for a selection. */
  totals: TotalKind;
  /** The totals the status bar shows side by side (Sum and Count unless
   *  others are ticked), in the order of TOTAL_KINDS. */
  totalsShown: TotalKind[];
  /** Alternating row colours for new tabs (each tab and view can differ). */
  banded: boolean;
  /** Suggest the rest of a value from the column while typing. */
  typeahead: boolean;
  /** Custom sort orders the user added. */
  sortLists: { name: string; items: string[] }[];
  /** Keep a recovery copy of unsaved changes, every `autosaveMinutes`. */
  autosave: boolean;
  autosaveMinutes: number;
  /** Keep each file as it was before a save (File > Earlier saves). Off
   *  unless chosen: these are extra copies of the user's data. (A new name:
   *  test builds that stored `keepVersions` with the old default of on
   *  start off too.) */
  keepEarlierSaves: boolean;
  /** Pick up where you left off: offer the last tabs at start ("ask"),
   *  reopen them without asking, or never remember them. */
  reopenLast: "ask" | "always" | "never";
  /** A file opened with no remembered title-row choice asks (in a bar)
   *  whether its first row holds the column names. */
  askHeader: boolean;
  /** Warn before private-looking data leaves the app (export, print, Save
   *  As, large copies). */
  privacyWarnings: boolean;
  /** Searches saved by name in the find bar (only when Save is pressed). */
  savedSearches: SavedSearch[];
}

export type TotalKind = "sum" | "average" | "count" | "numbers" | "min" | "max" | "distinct" | "chars";
export type EnterMove = "down" | "right" | "up" | "left" | "none";
export const ENTER_MOVES: EnterMove[] = ["down", "right", "up", "left", "none"];

/** The step Enter takes ([rows, columns]); Shift+Enter takes the opposite. */
export function enterStep(move: EnterMove, back: boolean): [number, number] {
  const s: [number, number] = move === "down" ? [1, 0] : move === "up" ? [-1, 0] : move === "right" ? [0, 1] : move === "left" ? [0, -1] : [0, 0];
  return back ? [-s[0], -s[1]] : s;
}

export const TOTAL_KINDS: TotalKind[] = ["sum", "average", "count", "numbers", "min", "max", "distinct", "chars"];

/** The saved list of status-bar totals, cleaned (unknown kinds dropped,
 *  in the usual order); Sum and Count when missing or empty. */
export function shownTotals(raw: unknown): TotalKind[] {
  if (!Array.isArray(raw)) return ["sum", "count"];
  const out = TOTAL_KINDS.filter((k) => raw.includes(k));
  return out.length ? out : ["sum", "count"];
}

/** The style value for a font family typed by name (checked). */
export function customFont(name: string): string | null {
  const f = safeFont(name);
  return f ? `"${f}", sans-serif` : null;
}

function fontOk(v: unknown): v is string {
  if (typeof v !== "string") return false;
  if (FONTS.some((f) => f.value === v)) return true;
  const m = /^"([^"]*)", sans-serif$/.exec(v);
  return !!m && safeFont(m[1]!) === m[1];
}

export const ACCENTS: { name: string; value: string }[] = [
  { name: "Teal", value: "#1f7a6d" },
  { name: "Blue", value: "#2d62c8" },
  { name: "Violet", value: "#7348c2" },
  { name: "Rose", value: "#b8406a" },
  { name: "Amber", value: "#a86411" },
  { name: "Slate", value: "#4c5a6e" },
];

export const FONTS: { name: string; value: string }[] = [
  { name: "System", value: '"Segoe UI Variable", "Segoe UI", system-ui, sans-serif' },
  { name: "Monospace", value: 'Consolas, "Cascadia Mono", ui-monospace, monospace' },
  { name: "Serif", value: 'Cambria, Georgia, "Times New Roman", serif' },
  { name: "Condensed", value: '"Arial Narrow", "Segoe UI", sans-serif' },
];

export const ZOOM_MIN = 0.5;
export const ZOOM_MAX = 3;

const KEY = "natendo.prefs.v1";

const DEFAULTS: PrefsData = {
  theme: "system",
  accent: ACCENTS[0]!.value,
  font: FONTS[0]!.value,
  fontSize: 13,
  zoom: 1,
  editPanel: false,
  gridLines: true,
  showInvisible: false,
  problemStyle: "fill",
  alignNumbers: true,
  colLetters: false,
  insights: false,
  enterMove: "down",
  listArrows: false,
  spellcheck: true,
  agentLink: false,
  shortcuts: {},
  views: [],
  panel: null,
  totals: "sum",
  totalsShown: ["sum", "count"],
  banded: false,
  typeahead: true,
  sortLists: [],
  autosave: true,
  autosaveMinutes: 2,
  keepEarlierSaves: false,
  reopenLast: "ask",
  askHeader: true,
  privacyWarnings: true,
  savedSearches: [],
};

function load(): PrefsData {
  try {
    return parsePrefs(localStorage.getItem(KEY));
  } catch {
    return { ...DEFAULTS };
  }
}

/** Saved preferences (untrusted text), checked, with defaults for the rest. */
export function parsePrefs(raw: string | null): PrefsData {
  try {
    if (!raw) return { ...DEFAULTS };
    const p = JSON.parse(raw) as Partial<PrefsData>;
    return {
      theme: p.theme === "light" || p.theme === "dark" ? p.theme : "system",
      accent: ACCENTS.some((a) => a.value === p.accent) ? p.accent! : DEFAULTS.accent,
      font: fontOk(p.font) ? p.font : DEFAULTS.font,
      fontSize: typeof p.fontSize === "number" ? Math.min(28, Math.max(9, p.fontSize)) : DEFAULTS.fontSize,
      zoom: typeof p.zoom === "number" ? Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, p.zoom)) : 1,
      editPanel: p.editPanel === true,
      gridLines: p.gridLines !== false,
      showInvisible: p.showInvisible === true,
      problemStyle: p.problemStyle === "underline" || p.problemStyle === "both" ? p.problemStyle : "fill",
      alignNumbers: p.alignNumbers !== false,
      colLetters: p.colLetters === true,
      insights: p.insights === true,
      listArrows: p.listArrows === true,
      enterMove: ENTER_MOVES.includes(p.enterMove as EnterMove) ? (p.enterMove as EnterMove) : "down",
      spellcheck: p.spellcheck !== false,
      agentLink: p.agentLink === true,
      shortcuts: p.shortcuts && typeof p.shortcuts === "object" ? p.shortcuts : {},
      views: Array.isArray(p.views) ? p.views.filter((v) => v && typeof v === "object" && Array.isArray(v.columns)).slice(0, MAX_VIEWS) : [],
      panel: p.panel === "summary" || p.panel === "chart" || p.panel === "files" || p.panel === "activity" || p.panel === "filters" || p.panel === "steps" ? p.panel : null,
      totals: TOTAL_KINDS.includes(p.totals as TotalKind) ? p.totals! : "sum",
      totalsShown: shownTotals(p.totalsShown),
      banded: p.banded === true,
      typeahead: p.typeahead !== false,
      sortLists: Array.isArray(p.sortLists) ? p.sortLists.filter((l) => l && typeof l.name === "string" && Array.isArray(l.items)).slice(0, 60).map((l) => ({ name: String(l.name).slice(0, 200), items: l.items.filter((i): i is string => typeof i === "string").slice(0, 10_000).map((i) => i.slice(0, 1000)) })) : [],
      autosave: p.autosave !== false,
      keepEarlierSaves: p.keepEarlierSaves === true,
      reopenLast: p.reopenLast === "always" || p.reopenLast === "never" ? p.reopenLast : "ask",
      askHeader: p.askHeader !== false,
      privacyWarnings: p.privacyWarnings !== false,
      savedSearches: savedSearches(p.savedSearches),
      autosaveMinutes: typeof p.autosaveMinutes === "number" && Number.isFinite(p.autosaveMinutes) ? Math.min(60, Math.max(1, Math.round(p.autosaveMinutes))) : 2,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

/** A search saved by name: the text, the replacement and the options. */
export interface SavedSearch {
  name: string;
  text: string;
  replacement: string;
  kind: "plain" | "wildcard" | "regex" | "number" | "date" | "similar";
  blank: boolean;
  matchCase: boolean;
  wholeCell: boolean;
  wholeWord: boolean;
  ignoreAccents: boolean;
  escapes: boolean;
  preserveCase: boolean;
  /** Dates are read day first (03/04/2026 is 3 April). */
  dayFirst: boolean;
}
export const MAX_SAVED_SEARCHES = 100;
const KINDS = ["plain", "wildcard", "regex", "number", "date", "similar"] as const;

/** Saved searches (untrusted), checked and bounded: a blank or repeated
 *  name is dropped (the first one with a name is kept). */
export function savedSearches(v: unknown): SavedSearch[] {
  if (!Array.isArray(v)) return [];
  const str = (x: unknown, n: number) => (typeof x === "string" ? x.slice(0, n) : "");
  const seen = new Set<string>();
  return v
    .filter((s): s is Record<string, unknown> => !!s && typeof s === "object" && typeof (s as { name?: unknown }).name === "string")
    .filter((s) => {
      const name = str(s.name, 100).trim();
      if (!name || seen.has(name)) return false;
      seen.add(name);
      return true;
    })
    .slice(0, MAX_SAVED_SEARCHES)
    .map((s) => ({
      name: str(s.name, 100).trim(),
      text: str(s.text, 1000),
      replacement: str(s.replacement, 1000),
      kind: (KINDS as readonly string[]).includes(s.kind as string) ? (s.kind as SavedSearch["kind"]) : "plain",
      blank: s.blank === true,
      matchCase: s.matchCase === true,
      wholeCell: s.wholeCell === true,
      wholeWord: s.wholeWord === true,
      ignoreAccents: s.ignoreAccents === true,
      escapes: s.escapes === true,
      preserveCase: s.preserveCase === true,
      dayFirst: s.dayFirst === true,
    }));
}

class Prefs {
  data = $state<PrefsData>(load());
  /** Called when the second window tried to change a setting (it writes none:
   *  they are changed in the main window). */
  onBlockedSave: (() => void) | null = null;

  constructor() {
    // The second window follows what the main window saves.
    if (SECOND_WINDOW && typeof window !== "undefined") {
      window.addEventListener("storage", (e) => {
        if (e.key === KEY) this.data = load();
      });
    }
  }

  save(): void {
    if (SECOND_WINDOW) {
      this.onBlockedSave?.();
      return;
    }
    try {
      localStorage.setItem(KEY, JSON.stringify(this.data));
    } catch {
      // Storage unavailable: keep the settings for this session only.
    }
  }

  setZoom(z: number): void {
    this.data.zoom = Math.round(Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z)) * 10) / 10;
    this.save();
  }
}

export const prefs = new Prefs();
