// Remembers, per file, whether its first row holds the column names, so a
// file opened again keeps the choice. Only a fingerprint of the path (a
// SHA-256, never the path itself) and the choice are kept, on this PC, for
// the 1,000 files used last. Nothing is kept for files in private mode.
// The fingerprint is salted with a random value made once on this PC, so a
// guessed path can't be checked against it.

const KEY = "nde.headers.v1";
const SALT_KEY = "nde.headers.salt";
let memSalt = "";

function salt(): string {
  try {
    let v = localStorage.getItem(SALT_KEY);
    if (!v || !/^[0-9a-f]{32}$/.test(v)) {
      v = randomHex();
      localStorage.setItem(SALT_KEY, v);
    }
    return v;
  } catch {
    return (memSalt ||= randomHex());
  }
}

function randomHex(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, "0")).join("");
}
/** Files remembered; the oldest are forgotten first. */
export const HEADER_MEMORY_MAX = 1000;

type Store = Record<string, [0 | 1, number]>;

function load(): Store {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Store) : {};
  } catch {
    return {};
  }
}

function store(s: Store): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // Not remembered this time.
  }
}

/** The fingerprint of a path: Windows paths ignore case and slash style. */
export async function pathKey(path: string): Promise<string> {
  const norm = path.replace(/\//g, "\\").toLowerCase();
  const d = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${salt()}\n${norm}`));
  return Array.from(new Uint8Array(d).slice(0, 16), (b) => b.toString(16).padStart(2, "0")).join("");
}

let clock = 0;
const now = () => Math.max(Date.now(), ++clock);

/** The choice remembered for this file, or null. */
export async function recallHeader(path: string): Promise<boolean | null> {
  const e = load()[await pathKey(path)];
  return Array.isArray(e) ? e[0] === 1 : null;
}

/** Remember the choice (or, for a private file, forget it). */
export async function rememberHeader(path: string, header: boolean, isPrivate = false): Promise<void> {
  const k = await pathKey(path);
  const s = load();
  if (isPrivate) {
    if (!(k in s)) return;
    delete s[k];
    store(s);
    return;
  }
  s[k] = [header ? 1 : 0, now()];
  const keys = Object.keys(s);
  if (keys.length > HEADER_MEMORY_MAX) {
    keys.sort((a, b) => (s[a]?.[1] ?? 0) - (s[b]?.[1] ?? 0));
    for (const old of keys.slice(0, keys.length - HEADER_MEMORY_MAX)) delete s[old];
  }
  store(s);
}

/** How many files' choices are remembered. */
export function rememberedHeaders(): number {
  return Object.keys(load()).length;
}

/** Forget every file's choice (and the salt: new fingerprints from now). */
export function forgetHeaders(): void {
  memSalt = "";
  try {
    localStorage.removeItem(KEY);
    localStorage.removeItem(SALT_KEY);
  } catch {
    // Nothing kept.
  }
}
