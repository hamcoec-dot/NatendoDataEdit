// Mailing labels and letters (Milestone 14): types shared with the engine,
// and the template reading the browser preview uses.
import type { Paper } from "./backend";

export type MailKind = "labels" | "letters";

/** A sheet of labels, sizes in inches. */
export interface LabelSheet {
  across: number;
  down: number;
  width: number;
  height: number;
  top: number;
  left: number;
  gap_across: number;
  gap_down: number;
  paper: Paper;
}

export interface MailSpec {
  kind: MailKind;
  /** Text with {Column title} fields; {{ and }} for braces. */
  template: string;
  header: boolean;
  sheet: LabelSheet;
  /** Labels already used on the first sheet. */
  skip: number;
  paper: Paper;
  margin_mm: number;
  font_size: number;
  skip_blank_lines: boolean;
  rows: [number, number] | null;
}

export interface MailReport {
  rows: number;
  pages: number;
  bytes: number;
  /** Lines cut short because they did not fit on a label. */
  cut: number;
}

/** The engine's first sheet, for the browser preview and as a default. */
export const DEFAULT_SHEET: LabelSheet = {
  across: 3,
  down: 10,
  width: 2.625,
  height: 1,
  top: 0.5,
  left: 0.1875,
  gap_across: 0.125,
  gap_down: 0,
  paper: "letter",
};

/** A first template from the column titles: name, street, city line. */
export function suggestTemplate(titles: string[]): string {
  const find = (...words: string[]) => titles.find((t) => words.some((w) => t.toLowerCase().replace(/[^a-z0-9]/g, "").includes(w)));
  const first = find("firstname", "first", "given");
  const last = find("lastname", "last", "surname", "family");
  const name = first || last ? [first, last].filter(Boolean).map((t) => `{${t}}`).join(" ") : find("name") ? `{${find("name")}}` : "";
  const street = find("address", "street", "addr1", "address1");
  const street2 = find("address2", "addr2", "unit", "apt");
  const city = find("city", "town");
  const state = titles.find((t) => ["state", "st", "province"].includes(t.toLowerCase().replace(/[^a-z0-9]/g, ""))) ?? find("state");
  const zip = find("zip", "postal", "postcode");
  const lines = [name, street ? `{${street}}` : "", street2 && street2 !== street ? `{${street2}}` : "", [city ? `{${city}}` : "", [state ? `{${state}}` : "", zip ? `{${zip}}` : ""].filter(Boolean).join(" ")].filter(Boolean).join(", ")];
  const out = lines.filter(Boolean).join("\n");
  return out || titles.slice(0, 3).map((t) => `{${t}}`).join("\n");
}

/** The fields a template uses that are not column titles (or numbers). */
export function unknownFields(template: string, titles: string[]): string[] {
  const out: string[] = [];
  const t = template.replace(/\{\{|\}\}/g, "");
  for (const m of t.matchAll(/\{([^{}\n]*)\}/g)) {
    const key = m[1]!.trim();
    const n = Number(key);
    const ok = titles.some((x) => x.trim().toLowerCase() === key.toLowerCase()) || (Number.isInteger(n) && n >= 1 && n <= titles.length);
    if (!ok && !out.includes(key)) out.push(key);
  }
  return out;
}

/** Fill a template for one row (browser preview; the engine does the real
 *  work and the same blank-line rule). */
export function fillTemplate(template: string, titles: string[], row: string[], skipBlank: boolean): string[] {
  const col = (key: string): number => {
    const i = titles.findIndex((x) => x.trim().toLowerCase() === key.trim().toLowerCase());
    return i >= 0 ? i : Number(key) - 1;
  };
  const out: string[] = [];
  for (const line of template.replace(/\r/g, "").split("\n")) {
    let fields = 0;
    let filled = 0;
    const text = line.replace(/\{\{|\}\}|\{([^{}]*)\}/g, (m, key?: string) => {
      if (m === "{{") return "{";
      if (m === "}}") return "}";
      fields++;
      const v = row[col(key ?? "")] ?? "";
      if (v.trim()) filled++;
      return v;
    });
    if (fields && skipBlank) {
      if (filled) out.push(text.split(" ").filter(Boolean).join(" ").replace(/,+$/, ""));
    } else out.push(text);
  }
  return out;
}
