// View › Show invisible characters: spaces, tabs and line breaks drawn as
// faint marks, and the characters that make two values look the same but not
// match (non-breaking, zero-width and other odd spaces) in a warning colour.
// Display only: the cell's value never changes.

/** A run of text drawn one way: plain text, a faint mark (space, tab, line
 *  break) or an odd character (the warning colour). */
export interface Run {
  text: string;
  kind: "text" | "mark" | "odd";
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
