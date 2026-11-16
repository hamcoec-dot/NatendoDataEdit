// Every user action is a command with an id. Menus, the toolbar, the
// right-click menu and keyboard shortcuts all run the same commands.

export interface Command {
  id: string;
  label: string;
  /** Shortcuts like "Ctrl+Shift+Z"; the first is shown in menus. */
  keys?: string[];
  run: () => unknown;
  enabled?: () => boolean;
  checked?: () => boolean;
  /** F4 ("Repeat") runs the last command marked repeatable again. */
  repeatable?: boolean;
}

export type MenuItem = string | "-";

export interface Menu {
  label: string;
  items: MenuItem[];
}

// In the installed app (WebView2 on Windows), a letter pressed while Alt is
// held can arrive with altKey off, typically for the second Alt+letter in a
// row (seen with real key presses; test tools always set altKey). Alt's own
// key events still arrive, so Alt is also tracked from them. While Alt is
// held Windows repeats its keydown every ~30 ms, so a stale state (a lost
// keyup) expires after ALT_TRUST_MS. AltGr (typing @, the euro sign and so
// on: it arrives as Ctrl+Alt) never counts. The guess is used only by
// keyOfLoose: the search toggle and Alt+letter menus outside text boxes.
const ALT_TRUST_MS = 600;
let altDownAt = 0;

function altGr(e: KeyboardEvent): boolean {
  return e.key === "AltGraph" || e.ctrlKey || !!e.getModifierState?.("AltGraph");
}

/** Track Alt from its own key events (call once, from the app's root). */
export function watchAlt(target: Pick<Window, "addEventListener" | "removeEventListener">): () => void {
  const down = (e: KeyboardEvent) => {
    // Alt itself, or a key that says Alt is down: Alt is held now.
    if (altGr(e)) altDownAt = 0;
    else if (e.key === "Alt" || e.altKey) altDownAt = e.timeStamp || performance.now();
  };
  const up = (e: KeyboardEvent) => {
    if (e.key === "Alt" || e.key === "AltGraph" || e.key === "Control") altDownAt = 0;
  };
  const reset = () => (altDownAt = 0);
  target.addEventListener("keydown", down, true);
  target.addEventListener("keyup", up, true);
  target.addEventListener("blur", reset);
  return () => {
    target.removeEventListener("keydown", down, true);
    target.removeEventListener("keyup", up, true);
    target.removeEventListener("blur", reset);
  };
}

/** Whether Alt is held for this key event, also when the event forgot to
 *  say so (see watchAlt). */
export function altOf(e: KeyboardEvent): boolean {
  if (e.altKey) return true;
  if (altDownAt === 0 || e.key === "Alt" || altGr(e)) return false;
  const now = e.timeStamp || performance.now();
  return now >= altDownAt && now - altDownAt < ALT_TRUST_MS;
}

/** Normalised shortcut text for a key event, e.g. "Ctrl+Shift+ArrowUp". */
export function keyOf(e: KeyboardEvent): string {
  return keyText(e, e.altKey);
}

/** keyOf, also counting an Alt the event forgot (only for the search toggle
 *  and Alt+letter menus, never for typing or recording shortcuts). */
export function keyOfLoose(e: KeyboardEvent): string {
  return keyText(e, altOf(e));
}

function keyText(e: KeyboardEvent, alt: boolean): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (alt) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  let k = e.key;
  if (k === " ") k = "Space";
  else if (k.length === 1) k = k.toUpperCase();
  if (k === "Control" || k === "Shift" || k === "Alt" || k === "Meta") return parts.join("+");
  parts.push(k);
  return parts.join("+");
}

export class Registry {
  private byId = new Map<string, Command>();
  private byKey = new Map<string, Command>();
  lastRepeatable: string | null = null;

  add(cmds: Command[]): void {
    for (const c of cmds) {
      if (this.byId.has(c.id)) throw new Error(`duplicate command ${c.id}`);
      this.byId.set(c.id, c);
      for (const k of c.keys ?? []) {
        if (this.byKey.has(k)) throw new Error(`shortcut ${k} used twice`);
        this.byKey.set(k, c);
      }
    }
  }

  get(id: string): Command | undefined {
    return this.byId.get(id);
  }

  forKey(key: string): Command | undefined {
    return this.byKey.get(key);
  }

  enabled(id: string): boolean {
    const c = this.byId.get(id);
    return !!c && (c.enabled?.() ?? true);
  }

  async run(id: string): Promise<boolean> {
    const c = this.byId.get(id);
    if (!c || !(c.enabled?.() ?? true)) return false;
    if (c.repeatable) this.lastRepeatable = id;
    await c.run();
    return true;
  }

  all(): Command[] {
    return [...this.byId.values()];
  }

  /** The keys each command had when added (Excel's, by default). */
  private defaults = new Map<string, string[]>();

  /** Use custom keys: `over[id]` replaces that command's keys, and a key
   *  given to one command is taken from any other that had it. Unknown
   *  commands are ignored. */
  setOverrides(over: Record<string, string[]>): void {
    if (!this.defaults.size) for (const c of this.byId.values()) this.defaults.set(c.id, [...(c.keys ?? [])]);
    const taken = new Set<string>();
    for (const [id, ks] of Object.entries(over)) if (this.byId.has(id)) ks.forEach((k) => taken.add(k));
    this.byKey.clear();
    for (const c of this.byId.values()) {
      const mine = over[c.id];
      const keys = mine ?? (this.defaults.get(c.id) ?? []).filter((k) => !taken.has(k));
      c.keys = keys.filter((k) => !this.byKey.has(k));
      for (const k of c.keys) this.byKey.set(k, c);
    }
  }

  /** A command's default keys. */
  defaultKeys(id: string): string[] {
    return this.defaults.get(id) ?? this.byId.get(id)?.keys ?? [];
  }
}
