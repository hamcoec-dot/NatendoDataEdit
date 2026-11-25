import { describe, expect, it } from "vitest";
import app from "../App.svelte?raw";

// Every command's shortcut, read from the command list in App.svelte: no two commands may share one (a clash
// means one of them quietly stops working, or runs the wrong command: Alt+Shift+Down was Move rows down).
describe("shortcuts", () => {
  const found: { id: string; key: string }[] = [];
  const re = /\{\s*id:\s*"([^"]+)"[^\n]*?keys:\s*\[([^\]]*)\]/g;
  for (const m of app.matchAll(re)) {
    for (const k of m[2]!.matchAll(/"([^"]+)"/g)) found.push({ id: m[1]!, key: k[1]!.toLowerCase().replace(/\s+/g, "") });
  }
  // Pairs that are the same key on purpose (a command and its alias, or one command in two modes).
  const ALLOWED = new Set<string>([]);

  it("finds the commands' keys", () => {
    expect(found.length).toBeGreaterThan(80);
    expect(found.some((f) => f.id === "data.diffNext" && f.key === "alt+f7")).toBe(true);
  });

  it("no two commands share a key", () => {
    const byKey = new Map<string, string[]>();
    for (const f of found) byKey.set(f.key, [...(byKey.get(f.key) ?? []), f.id]);
    const clashes = [...byKey].filter(([k, ids]) => new Set(ids).size > 1 && !ALLOWED.has(k)).map(([k, ids]) => `${k}: ${[...new Set(ids)].join(", ")}`);
    expect(clashes).toEqual([]);
  });
});
