// Format › Tidy column titles, as the engine does it (titles.rs), for the
// browser stub and tests. The desktop app always asks the engine.
import type { TitleSpec } from "./backend";

const MAX_TITLE_CHARS = 200;

function words(s: string): string[] {
  return s.split(/[\s_-]+/u).filter(Boolean);
}

function titleWord(w: string): string {
  const [first = "", ...rest] = [...w];
  return first.toUpperCase() + rest.join("").toLowerCase();
}

/** One title tidied (not made unique). */
export function tidyTitle(s: string, spec: TitleSpec): string {
  let t = spec.odd ? [...s].filter((c) => /[\p{L}\p{N}\s_-]/u.test(c)).join("") : s;
  if (spec.spaces) t = t.split(/\s+/u).filter(Boolean).join(" ");
  if (spec.case === "upper") t = t.toUpperCase();
  else if (spec.case === "lower") t = t.toLowerCase();
  else if (spec.case === "title") t = spec.spaces ? t.split(" ").map(titleWord).join(" ") : t.replace(/\S+/gu, titleWord);
  else if (spec.case === "snake") t = words(t).map((w) => w.toLowerCase()).join("_");
  return [...t].slice(0, MAX_TITLE_CHARS).join("");
}

/** Every title tidied; with `unique`, repeats (ignoring case) get a number
 *  and empty ones a name by place. */
export function tidyTitleList(titles: string[], spec: TitleSpec): string[] {
  const out = titles.map((t) => tidyTitle(t, spec));
  if (!spec.unique) return out;
  const sep = spec.case === "snake" ? "_" : " ";
  out.forEach((t, i) => {
    if (t === "") out[i] = spec.case === "snake" || spec.case === "lower" ? `column${sep}${i + 1}` : spec.case === "upper" ? `COLUMN ${i + 1}` : `Column ${i + 1}`;
  });
  const all = new Set(out.map((t) => t.toLowerCase()));
  const seen = new Set<string>();
  out.forEach((t, i) => {
    const k = t.toLowerCase();
    if (!seen.has(k)) {
      seen.add(k);
      return;
    }
    for (let n = 2; ; n++) {
      const cand = `${t}${sep}${n}`;
      const ck = cand.toLowerCase();
      if (!all.has(ck) && !seen.has(ck)) {
        seen.add(ck);
        out[i] = cand;
        return;
      }
    }
  });
  return out;
}
