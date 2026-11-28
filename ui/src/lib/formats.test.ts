// Time, date and time and custom display formats, and the format painter:
// display only (the file, its changed mark and its undo are untouched), one
// look step each, untrusted formats dropped, keys that never collide with typing.
import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { Tab, exportHandTab, importHandTab } from "./tab.svelte";
import { cleanFormat, defaultFormat, describeFormat, formatCell, formatCellColored, formatSteps, codeColorCss, isDarkColor, type DisplayFormat } from "./m9";
import { copyLook, pasteLook } from "./painter";
import { keyOf } from "./commands";

async function opened(): Promise<{ m: MockBackend; t: Tab }> {
  const m = new MockBackend();
  const s = await m.importText("when,phone,amount\n2026-03-14 14:30,6155551234,-1234.5\n9:05,123,0\n");
  const d = await m.importFinish(s.id, { format: "delimited", part: 0, raw_dates: false, record: "" });
  const t = new Tab(d, 100);
  t.header = true;
  t.baseline();
  return { m, t };
}

const custom = (code: string): DisplayFormat => ({ ...defaultFormat("custom"), code });

describe("time, date and time and custom display formats", () => {
  it("show cells without changing the file, the changed mark or undo, one look step each", async () => {
    const { m, t } = await opened();
    const before = await m.docInfo(t.id);
    const cells = async () => Promise.all([0, 1, 2].flatMap((r) => [0, 1, 2].map((c) => m.cellText(t.id, r, c))));
    const bytes = JSON.stringify(await cells());
    const formats: [number, DisplayFormat][] = [
      [0, defaultFormat("time")],
      [0, { ...defaultFormat("datetime"), time: "hm24" }],
      [1, custom("000-000-0000")],
      [2, custom("#,##0.00;[Red]-#,##0.00")],
    ];
    for (const [c, f] of formats) {
      t.columns.formats.set(c, f);
      t.recordView();
    }
    expect(t.lookSteps.length).toBe(4);
    t.columns.formats.delete(2);
    t.recordView();
    expect(t.lookSteps.length).toBe(5);
    const after = await m.docInfo(t.id);
    expect([after.modified, after.history_pos, after.can_undo]).toEqual([before.modified, before.history_pos, before.can_undo]);
    expect(JSON.stringify(await cells())).toBe(bytes);
    // Undo of the last look step puts the red code back.
    t.restoreView({ look: t.lookSteps.at(-1)!.before, sort: null }, false);
    expect(t.columns.formats.get(2)?.code).toBe("#,##0.00;[Red]-#,##0.00");
  });

  it("show as the shared rules say, and say what they are", () => {
    expect(formatCell("2026-03-14 14:30", defaultFormat("time"))).toBe("2:30 PM");
    expect(formatCell("2026-03-14", defaultFormat("time"))).toBe("2026-03-14");
    expect(formatCell("2026-03-14 14:30", defaultFormat("datetime"))).toBe("Mar 14, 2026 2:30 PM");
    expect(formatCell("2026-03-14", defaultFormat("datetime"))).toBe("Mar 14, 2026");
    expect(formatCell("9:05", defaultFormat("datetime"))).toBe("9:05 AM");
    expect(formatCell("14:30:59.9", { ...defaultFormat("time"), time: "hms24" })).toBe("14:30:59");
    expect(formatCell("6155551234", custom("000-000-0000"))).toBe("615-555-1234");
    expect(formatCellColored("-5", custom("0;[Red]-0"))).toEqual({ text: "-5", color: "red" });
    expect(formatCellColored("5", custom("0;[Red]-0"))).toEqual({ text: "5", color: null });
    expect(describeFormat(defaultFormat("time"))).toBe("time like 2:30 PM");
    expect(describeFormat(custom("0".repeat(45)))).toBe(`custom ${"0".repeat(40)}…`);
    expect(formatSteps(custom("00000"))).toEqual([{ tool: "format", code: "00000" }]);
    expect(formatSteps({ ...defaultFormat("datetime"), date: "iso", time: "hm24" })).toEqual([{ tool: "format", date: "yyyy-mm-dd", time: "hh:mm" }]);
  });

  it("draw [Red] in a readable shade, and no colour at all in high contrast", () => {
    expect(isDarkColor("#1e1e1e")).toBe(true);
    expect(isDarkColor("#ffffff")).toBe(false);
    expect(isDarkColor("rgb(30, 30, 30)")).toBe(true);
    expect(codeColorCss("red", false, false)).toBe("#c62828");
    expect(codeColorCss("red", true, false)).toBe("#ff7b7b");
    expect(codeColorCss("red", false, true)).toBeNull();
    expect(codeColorCss(null, false, false)).toBeNull();
  });

  it("are dropped, never thrown, when a view, the session or another window holds a code that cannot be shown", async () => {
    const bad = ["0".repeat(10_000), "0".repeat(201), "0;0;0;@;0", "[$-409]0", '"abc', "", 42, null];
    for (const code of bad) {
      expect(cleanFormat({ ...defaultFormat("custom"), code })).toBeNull();
    }
    expect(cleanFormat({ ...defaultFormat("custom"), code: "000-000-0000" })?.code).toBe("000-000-0000");
    // An older format (no time, no code) gets the defaults; a code on another kind is let go.
    expect(cleanFormat({ kind: "number", decimals: 2, thousands: true, symbol: "$", date: "iso" })).toEqual({ ...defaultFormat("number") });
    expect(cleanFormat({ ...defaultFormat("time"), code: "0" })?.code).toBe("");
    expect(cleanFormat({ ...defaultFormat("time"), time: "hm99" })?.time).toBe("hm12");
    // The other window's hand-over: a bad code there opens without that column's format.
    const { t } = await opened();
    t.columns.formats.set(1, custom("000-000-0000"));
    t.columns.formats.set(2, defaultFormat("time"));
    const h = exportHandTab(t);
    h.snap.look.cols.formats = h.snap.look.cols.formats.map(([c, f]) => [c, c === 1 ? { ...f, code: '"abc' } : f]);
    const { t: other } = await opened();
    importHandTab(other, JSON.parse(JSON.stringify(h)));
    expect(other.columns.formats.has(1)).toBe(false);
    expect(other.columns.formats.get(2)?.kind).toBe("time");
  });
});

describe("keys for the formats and the painter", () => {
  const ev = (o: Partial<KeyboardEvent> & { altGr?: boolean }): KeyboardEvent =>
    ({
      key: "",
      code: "",
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      ...o,
      getModifierState: (m: string) => m === "AltGraph" && !!o.altGr,
    }) as unknown as KeyboardEvent;

  it("Ctrl+Shift+2 is the 2 key on every layout", () => {
    // US: Shift+2 is @; UK and German: Shift+2 is ".
    expect(keyOf(ev({ key: "@", code: "Digit2", ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+2");
    expect(keyOf(ev({ key: '"', code: "Digit2", ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+2");
    // UK Shift+3 is £: Ctrl+Shift+3 still.
    expect(keyOf(ev({ key: "£", code: "Digit3", ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+3");
    // Without a key place (some test tools), the character as before.
    expect(keyOf(ev({ key: "@", ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+@");
  });

  it("an AltGr key press is never a shortcut", () => {
    // AltGr+Shift+C types Ć on Polish; AltGr+Q types @ on German: both arrive as Ctrl+Alt.
    expect(keyOf(ev({ key: "Ć", code: "KeyC", ctrlKey: true, altKey: true, shiftKey: true, altGr: true }))).toBe("");
    expect(keyOf(ev({ key: "@", code: "KeyQ", ctrlKey: true, altKey: true, altGr: true }))).toBe("");
    // Real Ctrl+Alt+Shift+C (left Ctrl and left Alt) is the shortcut.
    expect(keyOf(ev({ key: "C", code: "KeyC", ctrlKey: true, altKey: true, shiftKey: true }))).toBe("Ctrl+Alt+Shift+C");
    expect(keyOf(ev({ key: "V", code: "KeyV", ctrlKey: true, altKey: true, shiftKey: true }))).toBe("Ctrl+Alt+Shift+V");
  });
});

describe("the format painter", () => {
  it("copies one column onto several, and several repeating, as one look step", async () => {
    const { m, t } = await opened();
    const before = await m.docInfo(t.id);
    t.columns.formats.set(0, defaultFormat("time"));
    t.columns.aligns.set(0, "center");
    t.columns.wraps.add(0);
    t.columns.setWidth(0, 160);
    t.columns.formats.set(1, custom("000-000-0000"));
    t.columns.clips.add(1);
    t.recordView();
    const steps = t.lookSteps.length;
    t.columns.setCount(10);
    const one = copyLook(t.columns, [0]);
    expect(pasteLook(t.columns, [3, 4, 5], one)).toBe(3);
    t.recordView();
    expect(t.lookSteps.length).toBe(steps + 1);
    for (const c of [3, 4, 5]) {
      expect(t.columns.formats.get(c)?.kind).toBe("time");
      expect([t.columns.aligns.get(c), t.columns.wraps.has(c), t.columns.clips.has(c), t.columns.width(c)]).toEqual(["center", true, false, 160]);
    }
    const two = copyLook(t.columns, [0, 1]);
    pasteLook(t.columns, [6, 7, 8], two);
    expect([6, 7, 8].map((c) => t.columns.formats.get(c)?.kind)).toEqual(["time", "custom", "time"]);
    expect(t.columns.clips.has(7)).toBe(true);
    // A source with no format takes the target's away (the whole look).
    pasteLook(t.columns, [3], copyLook(t.columns, [2]));
    expect(t.columns.formats.has(3)).toBe(false);
    const after = await m.docInfo(t.id);
    expect([after.modified, after.history_pos, after.can_undo]).toEqual([before.modified, before.history_pos, before.can_undo]);
  });

  it("does nothing, and makes no step, when nothing changes or nothing was copied", async () => {
    const { t } = await opened();
    t.recordView();
    const steps = t.lookSteps.length;
    expect(pasteLook(t.columns, [1, 2], { cols: [] })).toBe(0);
    expect(pasteLook(t.columns, [], copyLook(t.columns, [0]))).toBe(0);
    expect(pasteLook(t.columns, [1], copyLook(t.columns, [0]))).toBe(0);
    t.recordView();
    expect(t.lookSteps.length).toBe(steps);
  });

  it("carries only the five parts: no text, no titles, no hidden state", async () => {
    const { t } = await opened();
    t.columns.formats.set(0, custom("00000"));
    const look = copyLook(t.columns, [0, 1, 2]);
    expect(Object.keys(look)).toEqual(["cols"]);
    for (const c of look.cols) expect(Object.keys(c).sort()).toEqual(["align", "clip", "format", "width", "wrap"]);
    const text = JSON.stringify(look);
    for (const cell of ["when", "phone", "amount", "6155551234", "2026-03-14"]) expect(text).not.toContain(cell);
    // A hidden target stays hidden; a hidden source gives its width from before.
    t.columns.setWidth(2, 0);
    pasteLook(t.columns, [2], copyLook(t.columns, [0]));
    expect(t.columns.isHidden(2)).toBe(true);
    expect(t.columns.formats.get(2)?.code).toBe("00000");
  });

  it("works on read-only and private tabs (a look is not data), across tabs", async () => {
    const { t: a } = await opened();
    const { m, t: b } = await opened();
    b.info = { ...b.info, read_only: true };
    b.private = { file: true, folder: false };
    a.columns.formats.set(1, custom("000-000-0000"));
    const before = await m.docInfo(b.id);
    pasteLook(b.columns, [1], copyLook(a.columns, [1]));
    expect(b.columns.formats.get(1)?.code).toBe("000-000-0000");
    // And back from the private tab to the other.
    a.columns.formats.delete(1);
    pasteLook(a.columns, [1], copyLook(b.columns, [1]));
    expect(a.columns.formats.get(1)?.code).toBe("000-000-0000");
    const after = await m.docInfo(b.id);
    expect([after.modified, after.history_pos]).toEqual([before.modified, before.history_pos]);
  });

  it("pastes onto 16,384 columns quickly, in one small look step", async () => {
    const { t } = await opened();
    t.columns.setCount(16_384);
    t.columns.formats.set(0, custom("#,##0.00;[Red]-#,##0.00"));
    t.columns.setWidth(0, 120);
    t.recordView();
    const steps = t.lookSteps.length;
    const targets = Array.from({ length: 16_384 }, (_, i) => i);
    const t0 = performance.now();
    pasteLook(t.columns, targets, copyLook(t.columns, [0]));
    t.recordView();
    const took = performance.now() - t0;
    expect(t.lookSteps.length).toBe(steps + 1);
    expect(took).toBeLessThan(500);
    // The look step's memory (counted as the look key's size): each distinct format once.
    expect(t.columns.key().length * 2).toBeLessThan(2 << 20);
  });
});
