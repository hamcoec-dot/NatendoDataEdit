// User preferences, kept in this machine's local storage. Reading or writing
// storage can fail (private mode, cleared data); defaults are used then.

import { MAX_VIEWS, safeFont, type Shortcuts, type View } from "./views";

export type Theme = "system" | "light" | "dark";

export interface PrefsData {
  theme: Theme;
  accent: string;
  font: string;
  fontSize: number;
  zoom: number;
  editPanel: boolean;
  gridLines: boolean;
  /** Underline misspelt words while editing cells. */
  spellcheck: boolean;
  /** The agent link was switched on (it starts again with the app). */
  agentLink: boolean;
  /** Custom keys by command id (cleaned against the commands at start). */
  shortcuts: Shortcuts;
  /** Saved views, by name. */
  views: View[];
  /** The side panel is open, and on which tab. */
  panel: "summary" | "chart" | "files" | "activity" | null;
  /** Which quick total the status bar shows for a selection. */
  totals: TotalKind;
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
  /** Warn before private-looking data leaves the app (export, print, Save
   *  As, large copies). */
  privacyWarnings: boolean;
}

export type TotalKind = "sum" | "average" | "count" | "numbers" | "min" | "max" | "distinct" | "chars";
export const TOTAL_KINDS: TotalKind[] = ["sum", "average", "count", "numbers", "min", "max", "distinct", "chars"];

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
  spellcheck: true,
  agentLink: false,
  shortcuts: {},
  views: [],
  panel: null,
  totals: "sum",
  banded: false,
  typeahead: true,
  sortLists: [],
  autosave: true,
  autosaveMinutes: 2,
  keepEarlierSaves: false,
  privacyWarnings: true,
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
      spellcheck: p.spellcheck !== false,
      agentLink: p.agentLink === true,
      shortcuts: p.shortcuts && typeof p.shortcuts === "object" ? p.shortcuts : {},
      views: Array.isArray(p.views) ? p.views.filter((v) => v && typeof v === "object" && Array.isArray(v.columns)).slice(0, MAX_VIEWS) : [],
      panel: p.panel === "summary" || p.panel === "chart" || p.panel === "files" || p.panel === "activity" ? p.panel : null,
      totals: TOTAL_KINDS.includes(p.totals as TotalKind) ? p.totals! : "sum",
      banded: p.banded === true,
      typeahead: p.typeahead !== false,
      sortLists: Array.isArray(p.sortLists) ? p.sortLists.filter((l) => l && typeof l.name === "string" && Array.isArray(l.items)).slice(0, 60).map((l) => ({ name: String(l.name).slice(0, 200), items: l.items.filter((i): i is string => typeof i === "string").slice(0, 10_000).map((i) => i.slice(0, 1000)) })) : [],
      autosave: p.autosave !== false,
      keepEarlierSaves: p.keepEarlierSaves === true,
      privacyWarnings: p.privacyWarnings !== false,
      autosaveMinutes: typeof p.autosaveMinutes === "number" && Number.isFinite(p.autosaveMinutes) ? Math.min(60, Math.max(1, Math.round(p.autosaveMinutes))) : 2,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

class Prefs {
  data = $state<PrefsData>(load());

  save(): void {
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
