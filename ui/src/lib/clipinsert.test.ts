import { describe, expect, it } from "vitest";
import { insertLabel, insertOffer, isInsertCopiedKey, type CopyRec } from "./clipinsert";

const key = (o: Partial<KeyboardEvent> & { altGr?: boolean }) => ({
  key: "",
  code: "",
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...o,
  getModifierState: (k: string) => k === "AltGraph" && !!o.altGr,
});

describe("the insert-copied key (review note 4)", () => {
  it("is Ctrl+Shift++ on the main keys or the number pad, and Ctrl+NumpadAdd", () => {
    expect(isInsertCopiedKey(key({ ctrlKey: true, shiftKey: true, key: "+", code: "Equal" }))).toBe(true);
    expect(isInsertCopiedKey(key({ ctrlKey: true, shiftKey: true, key: "+", code: "NumpadAdd" }))).toBe(true);
    expect(isInsertCopiedKey(key({ ctrlKey: true, key: "+", code: "NumpadAdd" }))).toBe(true);
  });
  it("is never AltGr, Alt, Ctrl+= or a + typed from another key", () => {
    expect(isInsertCopiedKey(key({ ctrlKey: true, altKey: true, shiftKey: true, key: "+", code: "Equal" }))).toBe(false);
    expect(isInsertCopiedKey(key({ ctrlKey: true, shiftKey: true, key: "+", code: "Equal", altGr: true }))).toBe(false);
    expect(isInsertCopiedKey(key({ ctrlKey: true, altKey: true, key: "+", code: "NumpadAdd" }))).toBe(false);
    expect(isInsertCopiedKey(key({ ctrlKey: true, key: "=", code: "Equal" }))).toBe(false);
    expect(isInsertCopiedKey(key({ ctrlKey: true, key: "+", code: "BracketRight" }))).toBe(false);
    expect(isInsertCopiedKey(key({ shiftKey: true, key: "+", code: "Equal" }))).toBe(false);
  });
});

const rec: CopyRec = { from: 1, kind: "rows", rect: { row: 2, rows: 3, col: 0, cols: 4 }, clip: "a\tb\r\nc\td\r\ne\tf\r\n", pos: 5, header: true, shut: false };

describe("the offer (review note 7)", () => {
  it("is there while nothing has changed the clipboard since the copy", () => {
    expect(insertOffer(rec, null)).toEqual({ ok: true, kind: "rows", count: 3 });
    expect(insertOffer(rec, true)).toEqual({ ok: true, kind: "rows", count: 3 });
    expect(insertLabel(insertOffer(rec, null))).toBe("Insert 3 copied rows above");
    expect(insertLabel(insertOffer({ ...rec, kind: "cols", rect: { ...rec.rect, cols: 1 } }, null))).toBe("Insert 1 copied column left");
  });
  it("is not there after another program put something on the clipboard, or with nothing copied", () => {
    const o = insertOffer(rec, false);
    expect(o.ok).toBe(false);
    if (!o.ok) expect(o.why).toBe("The clipboard holds something from another program");
    expect(insertOffer(null, null).ok).toBe(false);
  });
});
