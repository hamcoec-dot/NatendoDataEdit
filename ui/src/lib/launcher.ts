// Ranking for command search (Alt+Q). Each word typed must match the
// command's name, its menu, its README description or an everyday word for
// it ("dedupe", "vlookup"); small typing slips are forgiven. Pure and fast:
// a few hundred commands score in well under a millisecond per key press.

export interface LaunchItem {
  id: string;
  /** Name shown in the list (no & or …). */
  name: string;
  /** Menu it lives in, e.g. "Data". */
  menu?: string;
  /** README text describing it (for matching). */
  docs?: string;
}

/** Everyday words → words the app uses. */
export const SYNONYMS: Record<string, string[]> = {
  dedupe: ["duplicates"], dedup: ["duplicates"], deduplicate: ["duplicates"], dupe: ["duplicates"], dupes: ["duplicates"], unique: ["duplicates", "distinct"],
  vlookup: ["join"], xlookup: ["join"], hlookup: ["join"], lookup: ["join", "look"], index: ["join"], merge: ["join", "merge"], combine: ["merge", "combine", "append"],
  concat: ["merge"], concatenate: ["merge"], join: ["join", "merge"],
  delimiter: ["separator"], delimited: ["separator"], separator: ["separator", "split"], comma: ["separator"], tab: ["separator", "tab"],
  trim: ["spaces"], whitespace: ["spaces"], blanks: ["blank"], empty: ["blank"],
  uppercase: ["case"], lowercase: ["case"], upper: ["case"], lower: ["case"], capitalize: ["case"], capitalise: ["case"], caps: ["case"], proper: ["case"], titlecase: ["case"],
  regex: ["pattern", "patterns"], regexp: ["pattern", "patterns"], wildcard: ["find"], search: ["find", "search"], locate: ["find"], goto: ["go"],
  validate: ["check", "checks"], validation: ["checks"], verify: ["check"], errors: ["problem", "check"],
  transpose: ["swap", "paste"], rotate: ["swap", "pivot"], crosstab: ["pivot"], flatten: ["unpivot", "stack"],
  summarize: ["pivot", "summary"], summarise: ["pivot", "summary"], aggregate: ["pivot"], breakdown: ["pivot"], tally: ["pivot", "count"], drilldown: ["pivot"],
  formula: ["computed", "calculate"], formulas: ["computed"], calculate: ["calculate", "computed"], math: ["calculate"], expression: ["computed"],
  sum: ["total", "totals", "subtotals"], total: ["totals", "subtotals"], average: ["totals", "group"], count: ["count", "totals"], stats: ["summary"], statistics: ["summary"], profile: ["summary"],
  graph: ["chart"], plot: ["chart"], visualize: ["chart"], histogram: ["chart"],
  diff: ["compare", "changes"], compare: ["compare"], difference: ["compare"], differences: ["compare", "changes"], changed: ["changes", "edits"],
  history: ["earlier", "changes"], backup: ["earlier", "recover"], restore: ["recover", "revert"], crash: ["recover"], autosave: ["recover"],
  password: ["encrypted"], encrypt: ["encrypted"], decrypt: ["encrypted"], secure: ["encrypted", "private"],
  anonymize: ["mask", "scramble"], anonymise: ["mask", "scramble"], redact: ["mask"], obfuscate: ["scramble"], hide: ["hide", "mask"],
  pii: ["private"], ssn: ["private"], sensitive: ["private"], privacy: ["private"], personal: ["private"],
  macro: ["recipe"], macros: ["recipe"], automate: ["recipe", "watch"], automation: ["recipe"], batch: ["recipe", "folder"], script: ["recipe"], replay: ["recipe"],
  mailmerge: ["letters", "labels"], envelope: ["labels"], envelopes: ["labels"], letter: ["letters"],
  shortcut: ["shortcuts", "keys"], hotkey: ["shortcuts", "keys"], hotkeys: ["shortcuts", "keys"], keybinding: ["shortcuts"], keyboard: ["keyboard", "shortcuts"],
  dark: ["dark", "theme"], night: ["dark"], colour: ["colour", "colours"], color: ["colour", "colours"], colors: ["colours"], appearance: ["theme"],
  database: ["database"], sql: ["database"], sqlite: ["database"], db: ["database"],
  excel: ["import", "export"], xlsx: ["import", "export"], json: ["import", "export"], xml: ["import", "export"], html: ["import", "export", "html"], ods: ["import"], markdown: ["import", "export"],
  csv: ["save", "open"], tsv: ["save", "open"], txt: ["open"],
  random: ["random", "sample", "shuffle"], shuffle: ["shuffle"], subset: ["sample"],
  fuzzy: ["match"], nickname: ["match"], nicknames: ["match"], similar: ["match"], misspelled: ["match"],
  precinct: ["street"], district: ["street"], ward: ["street"], geocode: ["street"],
  mobile: ["phone"], telephone: ["phone"], postal: ["zip"], postcode: ["zip"], mail: ["mail", "email"], email: ["email", "mail"],
  delete: ["delete", "remove"], remove: ["remove", "delete"], erase: ["clear", "delete"], add: ["insert", "add"], insert: ["insert"], new: ["new", "insert"],
  rename: ["rename"], reorder: ["order", "move"], move: ["move", "order"], arrange: ["order"],
  lock: ["lock", "freeze"], readonly: ["lock"], freeze: ["freeze"], pin: ["freeze"],
  zoom: ["zoom"], bigger: ["larger", "zoom"], smaller: ["smaller", "zoom"], font: ["font", "text"],
  undo: ["undo"], redo: ["redo"], copy: ["copy"], paste: ["paste"],
  print: ["print"], pdf: ["pdf"], label: ["labels"],
  translate: ["values"], recode: ["values"], map: ["values"], cleanup: ["clean", "cleaning"], tidy: ["clean", "cleaning"], fix: ["fix", "clean"],
  date: ["date", "dates"], time: ["time", "dates"], calendar: ["date"], today: ["today", "date"],
  header: ["header", "title"], headers: ["header"], titles: ["header", "title"],
  conditional: ["highlight"], highlight: ["highlight"], accent: ["accents"], diacritics: ["accents"], url: ["web"], link: ["web"],
};

/** Everyday phrases → the app's words (checked before splitting into words). */
export const PHRASES: [string, string][] = [
  ["mail merge", "letters"],
  ["reorder rows", "move rows"],
  ["reorder columns", "move columns"],
  ["rearrange rows", "move rows"],
  ["rearrange columns", "move columns"],
  ["conditional formatting", "highlight rules"],
  ["conditional format", "highlight rules"],
  ["data validation", "data checks"],
  ["pivot table", "pivot"],
  ["group by", "pivot"],
  ["count by", "pivot"],
  ["sum by", "pivot"],
  ["count of each", "pivot"],
  ["drill down", "pivot"],
  ["cross tab", "pivot"],
  ["find and replace", "replace"],
  ["search and replace", "replace"],
  ["text to columns", "split column text"],
  ["text to column", "split column text"],
  ["insert row", "insert rows"],
  ["remove dupes", "remove duplicates"],
  ["title case", "case"],
  ["sentence case", "case"],
  ["upper case", "case"],
  ["lower case", "case"],
  ["dark mode", "theme dark"],
  ["light mode", "theme light"],
  ["word wrap", "wrap"],
  ["column width", "autofit"],
];

/** Lower case, accents off, punctuation to spaces. */
export function norm(s: string): string {
  return s
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/&/g, "")
    .replace(/[^a-z0-9+]+/g, " ")
    .trim();
}

/** Light stemming so "duplicates", "duplicate" and "duplicated" meet. */
export function stem(w: string): string {
  if (w.length > 5 && w.endsWith("ing")) return w.slice(0, -3);
  if (w.length > 4 && w.endsWith("ies")) return `${w.slice(0, -3)}y`;
  if (w.length > 4 && /(ches|shes|xes|ses)$/.test(w)) return w.slice(0, -2);
  if (w.length > 4 && w.endsWith("ed")) return w.slice(0, -2);
  if (w.length > 3 && w.endsWith("s") && !w.endsWith("ss")) w = w.slice(0, -1);
  if (w.length > 4 && w.endsWith("e")) return w.slice(0, -1);
  return w;
}

/** True when a and b differ by at most `max` single-letter edits
 *  (insert, delete, change, or swap of two neighbours). */
export function near(a: string, b: string, max: number): boolean {
  if (Math.abs(a.length - b.length) > max) return false;
  const n = b.length;
  let prev2: number[] = [];
  let prev = Array.from({ length: n + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    let low = i;
    for (let j = 1; j <= n; j++) {
      let v = Math.min(prev[j]! + 1, cur[j - 1]! + 1, prev[j - 1]! + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, prev2[j - 2]! + 1);
      cur.push(v);
      if (v < low) low = v;
    }
    if (low > max) return false;
    prev2 = prev;
    prev = cur;
  }
  return prev[n]! <= max;
}

const slips = (w: string): number => (w.length >= 7 ? 2 : w.length >= 4 ? 1 : 0);

interface Prepared {
  item: LaunchItem;
  name: string;
  nameWords: string[];
  nameStems: string[];
  initials: string;
  other: string[];
  otherStems: Set<string>;
  docWords: string[];
  docStems: Set<string>;
  menuStems: Set<string>;
}

export function prepare(items: LaunchItem[]): Prepared[] {
  return items.map((item) => {
    // Words in brackets ("URL encode (space → %20)") are side notes: they
    // count like the menu and id, not like the name itself.
    const name = norm(item.name.replace(/\([^)]*\)/g, " ")) || norm(item.name);
    const aside = norm((item.name.match(/\(([^)]*)\)/g) ?? []).join(" "));
    const nameWords = name.split(" ").filter(Boolean);
    const menuWords = norm(item.menu ?? "").split(" ").filter(Boolean);
    const other = [...norm(item.id.replace(/\./g, " ").replace(/([a-z])([A-Z])/g, "$1 $2")).split(" "), ...menuWords, ...aside.split(" ")].filter(Boolean);
    const docWords = [...new Set(norm(item.docs ?? "").split(" ").filter((w) => w.length > 1))];
    return {
      item,
      name,
      nameWords,
      nameStems: nameWords.map(stem),
      initials: nameWords.map((w) => w[0]).join(""),
      other,
      otherStems: new Set(other.map(stem)),
      docWords,
      docStems: new Set(docWords.map(stem)),
      menuStems: new Set(menuWords.map(stem)),
    };
  });
}

/** How well one typed word matches a command (0 = not at all). */
function wordScore(p: Prepared, w: string): number {
  const s = stem(w);
  let best = 0;
  for (let i = 0; i < p.nameWords.length; i++) {
    const nw = p.nameWords[i]!;
    if (nw === w || p.nameStems[i] === s) return 10;
    if (nw.startsWith(w)) best = Math.max(best, w.length === 1 ? 6 : 8);
    else if (w.length >= 3 && nw.includes(w)) best = Math.max(best, 5);
    else if (slips(w) && (near(w, nw, slips(w)) || (nw.length > w.length && near(w, nw.slice(0, w.length), slips(w))))) best = Math.max(best, 5);
  }
  if (best >= 8) return best;
  if (w.length >= 2 && p.initials.startsWith(w)) best = Math.max(best, 7);
  if (p.otherStems.has(s)) best = Math.max(best, 6);
  else if (w.length >= 3 && p.other.some((o) => o.startsWith(w))) best = Math.max(best, 4);
  if (best >= 4) return best;
  if (p.docStems.has(s)) return 3;
  if (w.length >= 3 && p.docWords.some((d) => d.startsWith(w))) return 2;
  if (slips(w) && p.docWords.some((d) => near(w, d, slips(w)))) return 1.5;
  return best;
}

/** Score for a typed word, trying its everyday-word meanings too. */
function termScore(p: Prepared, w: string, alts: string[]): number {
  let best = wordScore(p, w);
  // The first meaning listed is the main one; later ones count a little less.
  alts.forEach((alt, i) => {
    if (alt !== w) best = Math.max(best, wordScore(p, alt) * Math.max(0.6, 0.8 - 0.05 * i));
  });
  return best;
}

const synonymKeys = Object.keys(SYNONYMS);

/** Everyday-word meanings of a typed word. The last word may still be
 *  half typed, so it also takes the meanings of words it starts. */
function meanings(w: string, last: boolean): string[] {
  const out = new Set(SYNONYMS[w] ?? SYNONYMS[stem(w)] ?? []);
  if (last && w.length >= 3) for (const k of synonymKeys) if (k.length > w.length && k.startsWith(w)) for (const v of SYNONYMS[k]!) out.add(v);
  return [...out];
}

/** Swap everyday phrases for the app's words; a phrase still being typed
 *  at the end ("dark mo") counts once its first word is complete. */
function phrases(q: string): string {
  for (const [from, to] of PHRASES) if (q.includes(from)) q = q.replace(from, to);
  const words = q.split(" ");
  for (let k = 0; k < words.length - 1; k++) {
    const tail = words.slice(k).join(" ");
    const hit = PHRASES.find(([from]) => from.startsWith(tail));
    if (hit) return [...words.slice(0, k), hit[1]].join(" ");
  }
  return q;
}

export interface LaunchHit {
  item: LaunchItem;
  score: number;
}

/** Commands matching `query`, best first. An empty query lists them all by name. */
export function rank(prepared: Prepared[], query: string, limit = 50): LaunchHit[] {
  const q = phrases(norm(query));
  if (!q) return prepared.map((p) => ({ item: p.item, score: 0 })).sort((a, b) => a.item.name.localeCompare(b.item.name)).slice(0, limit);
  const words = q.split(" ").filter(Boolean);
  const alts = words.map((w, i) => meanings(w, i === words.length - 1));
  const hits: LaunchHit[] = [];
  for (const p of prepared) {
    let score = 0;
    let ok = true;
    for (let i = 0; i < words.length; i++) {
      const t = termScore(p, words[i]!, alts[i]!);
      if (t <= 0) {
        ok = false;
        break;
      }
      score += t;
    }
    if (!ok) continue;
    // A typed word naming the command's menu ("insert row", "add column")
    // tips the balance toward commands in that menu.
    if (words.some((w, i) => p.menuStems.has(stem(w)) || alts[i]!.some((a) => p.menuStems.has(stem(a))))) score += 4;
    if (p.name.startsWith(q)) score += 15;
    else if (p.name.includes(q)) score += 8;
    score -= p.name.length * 0.01;
    hits.push({ item: p.item, score });
  }
  return hits.sort((a, b) => b.score - a.score || a.item.name.localeCompare(b.item.name)).slice(0, limit);
}
