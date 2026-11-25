// View › Show invisible characters: spaces, tabs and line breaks drawn as
// faint marks, and the characters that make two values look the same but not
// match (non-breaking, zero-width and other odd spaces) in a warning colour.
// Display only: the cell's value never changes.

/** A run of text drawn one way: plain text, a faint mark (space, tab, line
 *  break) or an odd character (the warning colour). */
export interface Run {
  text: string;
  kind: "text" | "mark" | "odd";
  /** X-ray: a gremlin, drawn with a glow. */
  glow?: boolean;
}

/** Odd characters: they look like a space (or nothing) but are not one. */
const ODD = new Map<string, [mark: string, name: string]>([
  [" ", ["°", "non-breaking space"]],
  [" ", ["°", "figure space"]],
  [" ", ["°", "narrow non-breaking space"]],
  [" ", ["°", "unusual space"]],
  ["　", ["°", "wide space"]],
  ["​", ["¦", "zero-width space"]],
  ["‌", ["¦", "zero-width non-joiner"]],
  ["‍", ["¦", "zero-width joiner"]],
  ["⁠", ["¦", "word joiner"]],
  ["﻿", ["¦", "byte order mark"]],
  ["­", ["¬", "soft hyphen"]],
]);
// En, em, thin and other typesetting spaces.
for (let c = 0x2000; c <= 0x200a; c++) if (c !== 0x2007) ODD.set(String.fromCharCode(c), ["°", "unusual space"]);

/** A control character other than tab and line breaks. */
const isControl = (ch: string) => (ch < " " && ch !== "\t" && ch !== "\r" && ch !== "\n") || ch === "\u007f";

const ANY = /[\s\u0000-\u001f\u007f­​-‍⁠﻿]/;

/** The text with its invisible characters shown, as runs. Plain spaces are
 *  `·`, tabs `→`, line breaks `⏎` (faint marks); odd characters are `°`
 *  (wide or non-breaking spaces), `¦` (zero-width), `¬` (soft hyphen) or
 *  `¿` (control characters). */
export function invisibleRuns(s: string): Run[] {
  if (!ANY.test(s)) return [{ text: s, kind: "text" }];
  const out: Run[] = [];
  const push = (text: string, kind: Run["kind"]) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind) last.text += text;
    else out.push({ text, kind });
  };
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]!;
    const odd = ODD.get(ch);
    if (ch === " ") push("·", "mark");
    else if (ch === "\t") push("→", "mark");
    else if (ch === "\r" && s[i + 1] === "\n") {
      push("⏎", "mark");
      i++;
    } else if (ch === "\n" || ch === "\r") push("⏎", "mark");
    else if (odd) push(odd[0], "odd");
    else if (isControl(ch)) push("¿", "odd");
    else push(ch, "text");
  }
  return out;
}

/** The text with its invisible characters shown, in one string. */
export function invisibleText(s: string): string {
  return invisibleRuns(s)
    .map((r) => r.text)
    .join("");
}

const plural = (n: number, one: string) => `${n} ${one}${n > 1 ? "s" : ""}`;

/** A short note on what a value holds that can't be seen: "2 spaces at the
 *  end, 1 non-breaking space". Empty when there is nothing to say. */
export function invisibleNote(s: string): string {
  const parts: string[] = [];
  if (s !== "" && /^ +$/.test(s)) parts.push(`only spaces (${s.length})`);
  else {
    const lead = /^ +/.exec(s)?.[0].length ?? 0;
    const trail = / +$/.exec(s)?.[0].length ?? 0;
    if (lead) parts.push(plural(lead, "space") + " at the start");
    if (trail) parts.push(plural(trail, "space") + " at the end");
    const doubles = (s.trim().match(/ {2,}/g) ?? []).length;
    if (doubles) parts.push(plural(doubles, "double space"));
  }
  const tabs = (s.match(/\t/g) ?? []).length;
  if (tabs) parts.push(plural(tabs, "tab"));
  const breaks = (s.match(/\r\n|\r|\n/g) ?? []).length;
  if (breaks) parts.push(plural(breaks, "line break"));
  const counts = new Map<string, number>();
  for (const ch of s) {
    const name = ODD.get(ch)?.[1] ?? (isControl(ch) ? "control character" : "");
    if (name) counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  for (const [name, n] of counts) parts.push(plural(n, name));
  return parts.join(", ");
}

// ---- X-ray gremlins (View > X-ray gremlins) -------------------------------------------------------
// Everything above, plus smart quotes, tabs and line breaks inside a cell, and look-alike letters hiding in a
// word ("Smаrt" with a Cyrillic а). The characters come from the engine's own table
// (crates/engine/src/testdata/gremlins.json) and both copies pass the same corpus (xray.test.ts, xray.rs).
// A look-alike is flagged only inside a MIXED-script run: an unbroken run of letters (a hyphen, an apostrophe,
// a digit or a space ends the run) holding Latin letters and a Cyrillic or Greek letter that has a Latin
// twin in the table. A run of one script, or a letter with no twin, is never flagged.
// Display only: the cell's value never changes.

import GREMLINS from "../../../crates/engine/src/testdata/gremlins.json";

export type GremlinKind = "zero_width" | "odd_space" | "control" | "smart_quote" | "tab" | "line_break" | "homoglyph";

export interface Gremlin {
  kind: GremlinKind;
  /** Index in the string (UTF-16 units). */
  at: number;
  ch: string;
  /** What it becomes when fixed ("" : it is removed). */
  twin: string;
}

const CONFUSABLE = new Map<string, string>((GREMLINS.confusables as string[][]).map((p) => [p[0]!, p[1]!]));
const QUOTE = new Map<string, string>((GREMLINS.quotes as string[][]).map((p) => [p[0]!, p[1]!]));
const ODD_KIND = new Map<string, GremlinKind>((GREMLINS.invisible as string[][]).map((p) => [p[0]!, p[2] === "zero" ? "zero_width" : "odd_space"]));

type Script = "latin" | "cyrillic" | "greek";
function scriptOf(cp: number): Script | null {
  if ((cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a) || (cp >= 0xc0 && cp <= 0x24f) || (cp >= 0x1e00 && cp <= 0x1eff)) return "latin";
  if ((cp >= 0x370 && cp <= 0x3ff) || (cp >= 0x1f00 && cp <= 0x1fff)) return "greek";
  if (cp >= 0x400 && cp <= 0x52f) return "cyrillic";
  return null;
}
const isMark = (cp: number) => (cp >= 0x300 && cp <= 0x36f) || (cp >= 0x483 && cp <= 0x489) || (cp >= 0x1ab0 && cp <= 0x1aff) || (cp >= 0x1dc0 && cp <= 0x1dff) || (cp >= 0x20d0 && cp <= 0x20ff);
const ALPHA = /^\p{Alphabetic}$/u;
const QUICK = /[^\x20-\x7e]/;

const KIND_NAME: Record<GremlinKind, string> = {
  zero_width: "zero-width character",
  odd_space: "odd space",
  control: "control character",
  smart_quote: "smart quote",
  tab: "tab",
  line_break: "line break",
  homoglyph: "look-alike letter",
};

/** Every flagged character of `s`, in order. */
export function scanGremlins(s: string): Gremlin[] {
  if (!QUICK.test(s)) return [];
  const out: Gremlin[] = [];
  let run: { at: number; ch: string }[] = [];
  const flush = () => {
    if (run.length && run.some((r) => scriptOf(r.ch.codePointAt(0)!) === "latin")) {
      for (const r of run) {
        const sc = scriptOf(r.ch.codePointAt(0)!);
        if (sc === "cyrillic" || sc === "greek") {
          const twin = CONFUSABLE.get(r.ch);
          if (twin !== undefined) out.push({ kind: "homoglyph", at: r.at, ch: r.ch, twin });
        }
      }
    }
    run = [];
  };
  let at = 0;
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    if (ALPHA.test(ch) || (isMark(cp) && run.length)) {
      run.push({ at, ch });
    } else {
      flush();
      let g: Gremlin | null = null;
      const odd = ODD_KIND.get(ch);
      if (ch === "\t") g = { kind: "tab", at, ch, twin: " " };
      else if (ch === "\n" || ch === "\r") g = { kind: "line_break", at, ch, twin: " " };
      else if (odd) g = { kind: odd, at, ch, twin: odd === "odd_space" ? " " : "" };
      else if (QUOTE.has(ch)) g = { kind: "smart_quote", at, ch, twin: QUOTE.get(ch)! };
      else if ((cp < 0x20 && ch !== "\t" && ch !== "\n" && ch !== "\r") || cp === 0x7f) g = { kind: "control", at, ch, twin: "" };
      if (g) out.push(g);
    }
    at += ch.length;
  }
  flush();
  return out.sort((a, b) => a.at - b.at);
}

export interface SanitizeOptions {
  zero_width: boolean;
  odd_spaces: boolean;
  smart_quotes: boolean;
  tabs_breaks: boolean;
  homoglyphs: boolean;
}
/** The ticked options of the dialog: everything except tabs and line breaks. */
export const USUAL_SANITIZE: SanitizeOptions = { zero_width: true, odd_spaces: true, smart_quotes: true, tabs_breaks: false, homoglyphs: true };

/** `s` with the chosen gremlins fixed, or null when nothing changes (the engine's `sanitize`). */
export function sanitizeText(s: string, o: SanitizeOptions): string | null {
  const found = scanGremlins(s);
  if (!found.length) return null;
  let out = "";
  let last = 0;
  let changed = false;
  for (const g of found) {
    const fix =
      g.kind === "zero_width" ? o.zero_width : g.kind === "odd_space" ? o.odd_spaces : g.kind === "smart_quote" ? o.smart_quotes : g.kind === "tab" || g.kind === "line_break" ? o.tabs_breaks : g.kind === "homoglyph" ? o.homoglyphs : false;
    if (!fix) continue;
    out += s.slice(last, g.at) + g.twin;
    last = g.at + g.ch.length;
    changed = true;
  }
  if (!changed) return null;
  out += s.slice(last);
  if (o.tabs_breaks && s.includes("\r\n")) out = out.replaceAll("  ", " ");
  return out;
}

/** The text with its gremlins glowing, as runs (plain spaces, tabs and breaks as faint marks like Show invisible). */
export function xrayRuns(s: string): Run[] {
  const found = scanGremlins(s);
  if (!found.length && !ANY.test(s)) return [{ text: s, kind: "text" }];
  const at = new Map(found.map((g) => [g.at, g]));
  const out: Run[] = [];
  const push = (text: string, kind: Run["kind"], glow: boolean) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind && !!last.glow === glow) last.text += text;
    else out.push({ text, kind, glow: glow || undefined });
  };
  let i = 0;
  while (i < s.length) {
    const ch = String.fromCodePoint(s.codePointAt(i)!);
    const g = at.get(i);
    const odd = ODD.get(ch);
    if (g) {
      if (g.kind === "homoglyph" || g.kind === "smart_quote") push(ch, "text", true);
      else if (g.kind === "tab") push("→", "mark", true);
      else if (g.kind === "line_break") {
        push("⏎", "mark", true);
        if (ch === "\r" && s[i + 1] === "\n") i++;
      } else if (g.kind === "control") push("¿", "odd", true);
      else push(odd?.[0] ?? "¦", "odd", true);
    } else if (ch === " ") push("·", "mark", false);
    else if (odd) push(odd[0], "odd", false);
    else if (isControl(ch)) push("¿", "odd", false);
    else push(ch, "text", false);
    i += ch.length;
  }
  return out;
}

const plainName = (c: string) => (c >= "Ѐ" && c <= "ԯ" ? "Cyrillic" : "Greek");

/** A short note on the gremlins of a value: "Cyrillic а (U+0430) inside a Latin word, 1 smart quote". */
export function xrayNote(s: string): string {
  const found = scanGremlins(s);
  if (!found.length) return "";
  const parts: string[] = [];
  const counts = new Map<string, number>();
  for (const g of found) {
    if (g.kind === "homoglyph") {
      parts.push(`${plainName(g.ch)} ${g.ch} (U+${g.ch.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}) inside a Latin word`);
    } else {
      const name = g.kind === "odd_space" ? (ODD.get(g.ch)?.[1] ?? KIND_NAME.odd_space) : g.kind === "zero_width" ? (ODD.get(g.ch)?.[1] ?? KIND_NAME.zero_width) : KIND_NAME[g.kind];
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }
  for (const [name, n] of counts) parts.push(plural(n, name));
  return parts.join(", ");
}
