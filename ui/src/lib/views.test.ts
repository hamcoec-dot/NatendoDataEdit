import { describe, expect, it } from "vitest";
import { Registry } from "./commands";
import { cleanShortcuts, fmt, matchColumns, MAX_BINDINGS, safeFont, validKey } from "./views";

describe("views", () => {
  it("matches columns by title, then by place", () => {
    const view = [
      { title: "Name", width: 100, hidden: false },
      { title: "City", width: 80, hidden: true },
      { title: "Gone", width: null, hidden: false },
    ];
    expect(matchColumns(view, [" city", "NAME", "Zip"])).toEqual([1, 0, 2]);
    expect(matchColumns(view, ["Name"])).toEqual([0, null, null]);
  });

  it("checks font names", () => {
    expect(safeFont(" Fira Sans ")).toBe("Fira Sans");
    expect(safeFont('Arial", x')).toBeNull();
    expect(safeFont("a;b{}")).toBeNull();
    expect(safeFont("x".repeat(61))).toBeNull();
    expect(safeFont("")).toBeNull();
  });

  it("cleans custom shortcuts", () => {
    const known = (id: string) => id.startsWith("ok.");
    expect(cleanShortcuts({ "ok.a": ["Ctrl+J", 5, "<b>"], "bad.b": ["Ctrl+K"], "ok.c": "Ctrl+L" }, known)).toEqual({ "ok.a": ["Ctrl+J"] });
    expect(cleanShortcuts(null, known)).toEqual({});
    const many: Record<string, string[]> = {};
    for (let i = 0; i < 400; i++) many[`ok.${i}`] = ["Ctrl+J", "Ctrl+K"];
    const out = cleanShortcuts(many, known);
    expect(Object.values(out).flat().length).toBeLessThanOrEqual(MAX_BINDINGS);
    expect(validKey("Ctrl+Shift+F2")).toBe(true);
    expect(validKey("x".repeat(100))).toBe(false);
    // Typing and moving keys need Ctrl or Alt; Escape is never taken.
    for (const k of ["A", "Shift+A", "1", "Space", "Enter", "Tab", "Shift+Tab", "ArrowDown", "Shift+ArrowUp", "Delete", "Backspace", "Home", "Escape", "Ctrl+Escape"]) {
      expect(validKey(k), k).toBe(false);
    }
    for (const k of ["Ctrl+A", "Alt+Shift+A", "Ctrl+Enter", "Alt+ArrowDown", "F2", "Shift+F7"]) {
      expect(validKey(k), k).toBe(true);
    }
  });

  it("formats numbers", () => {
    expect(fmt(null)).toBe("—");
    expect(fmt(4999950000).replace(/\D/g, "")).toBe("4999950000");
    expect(fmt(2)).toContain("2");
    // Rounding noise from adding many decimals is not shown.
    expect(fmt(999999.9999997595).replace(/\D/g, "")).toBe("1000000");
    expect(fmt(0.1 + 0.2).replace(/\D/g, "")).toBe("03");
  });
});

describe("custom keys", () => {
  const make = () => {
    const r = new Registry();
    r.add([
      { id: "a", label: "A", keys: ["Ctrl+A"], run: () => {} },
      { id: "b", label: "B", keys: ["Ctrl+B"], run: () => {} },
      { id: "c", label: "C", run: () => {} },
    ]);
    return r;
  };

  it("replaces a command's keys and takes a key from its old owner", () => {
    const r = make();
    r.setOverrides({ c: ["Ctrl+B"] });
    expect(r.forKey("Ctrl+B")?.id).toBe("c");
    expect(r.get("b")?.keys).toEqual([]);
    expect(r.forKey("Ctrl+A")?.id).toBe("a");
    expect(r.defaultKeys("b")).toEqual(["Ctrl+B"]);
  });

  it("goes back to the defaults", () => {
    const r = make();
    r.setOverrides({ a: [] });
    expect(r.forKey("Ctrl+A")).toBeUndefined();
    r.setOverrides({});
    expect(r.forKey("Ctrl+A")?.id).toBe("a");
    expect(r.forKey("Ctrl+B")?.id).toBe("b");
  });

  it("ignores unknown commands", () => {
    const r = make();
    r.setOverrides({ nope: ["Ctrl+A"] });
    expect(r.forKey("Ctrl+A")?.id).toBe("a");
  });
});
