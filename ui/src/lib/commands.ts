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

/** Normalised shortcut text for a key event, e.g. "Ctrl+Shift+ArrowUp". */
export function keyOf(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
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
