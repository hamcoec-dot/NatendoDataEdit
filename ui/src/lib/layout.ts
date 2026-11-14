// Plain-words names for file layouts (separator, quotes, lines above the
// table), and turning what the user types into a separator.

import type { Layout } from "./backend";

const NAMES: Record<string, string> = {
  ",": "Comma",
  "\t": "Tab",
  ";": "Semicolon",
  "|": "Pipe",
  " ": "Space",
  ":": "Colon",
  "^": "Caret",
  "~": "Tilde",
  "\x1f": "Unit separator (0x1F)",
  "\x1e": "Record separator (0x1E)",
  "\x01": "Control-A (0x01)",
};

/** The separator in words. */
export function separatorName(l: Pick<Layout, "delimiter" | "merge">): string {
  if (l.merge && l.delimiter === " ") return "Spaces (lined-up columns)";
  const known = NAMES[l.delimiter];
  if (known) return known;
  return `“${visible(l.delimiter)}”`;
}

/** Control characters shown as \xNN, everything else as is. */
export function visible(s: string): string {
  return [...s]
    .map((c) => {
      const n = c.charCodeAt(0);
      if (c === "\t") return "\\t";
      return n < 0x20 ? `\\x${n.toString(16).toUpperCase().padStart(2, "0")}` : c;
    })
    .join("");
}

/** Read a separator typed by the user: `\t` and `\xNN` are understood. */
export function parseSeparator(typed: string): string {
  return typed
    .replace(/\\x([0-9a-fA-F]{2})/g, (_, h: string) => String.fromCharCode(parseInt(h, 16)))
    .replace(/\\t/g, "\t");
}

/** One line describing a layout. */
export function describeLayout(l: Layout, columns?: number): string {
  if (columns === 1) {
    const above = aboveText(l.skip_lines);
    return `One column (no separator)${above ? `, ${above}` : ""}`;
  }
  const parts = [separatorName(l)];
  if (l.quote === '"') parts.push("double quotes");
  else if (l.quote === "'") parts.push("single quotes");
  else if (l.quote) parts.push(`quote ${visible(l.quote)}`);
  else parts.push("no quotes");
  if (l.escape) parts.push(`${l.escape === "\\" ? "backslash" : visible(l.escape)} escapes`);
  const above = aboveText(l.skip_lines);
  if (above) parts.push(above);
  let text = parts.join(", ");
  if (columns !== undefined) text += ` (${columns.toLocaleString()} columns)`;
  return text;
}

function aboveText(n: number): string {
  if (n <= 0) return "";
  return `${n} line${n === 1 ? "" : "s"} above the table set aside`;
}

/** How sure detection was, in words. */
export function confidenceText(c: number): string {
  if (c >= 0.85) return "very likely";
  if (c >= 0.6) return "likely";
  if (c >= 0.35) return "a guess";
  return "a rough guess";
}

export function sameLayout(a: Layout, b: Layout): boolean {
  return (
    a.delimiter === b.delimiter &&
    a.quote === b.quote &&
    a.escape === b.escape &&
    a.merge === b.merge &&
    a.skip_lines === b.skip_lines
  );
}
