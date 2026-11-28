// Gap #6: several blocks dropped on another tab, and drops between two windows
// (the stub's two "windows" are two mock backends sharing a BroadcastChannel).
import { afterEach, describe, expect, it } from "vitest";
import { lineUp, pack } from "./drops";
import { MockBackend } from "./mock";
import type { DropNotice } from "./backend";

const r = (row: number, rows: number, col: number, cols: number) => ({ row, rows, col, cols });
const wait = (ms: number) => new Promise((res) => setTimeout(res, ms));

describe("several blocks", () => {
  it("line up as for Copy, or are refused", () => {
    expect(lineUp([r(1, 3, 4, 1), r(1, 3, 0, 2)])).toEqual({ areas: [r(1, 3, 0, 2), r(1, 3, 4, 1)], side: true });
    expect(lineUp([r(9, 2, 0, 2), r(1, 3, 0, 2)])).toEqual({ areas: [r(1, 3, 0, 2), r(9, 2, 0, 2)], side: false });
    expect(lineUp([r(1, 3, 0, 2), r(9, 2, 1, 2)])).toBeNull();
    expect(lineUp([r(2, 2, 2, 2)])).toEqual({ areas: [r(2, 2, 2, 2)], side: true });
  });

  it("go in packed as Copy packs them", () => {
    const a = [["a1", "a2"], ["b1", "b2"]];
    const b = [["x"], ["y"]];
    expect(pack([a, b], true)).toEqual([["a1", "a2", "x"], ["b1", "b2", "y"]]);
    expect(pack([a, [["c1", "c2"]]], false)).toEqual([["a1", "a2"], ["b1", "b2"], ["c1", "c2"]]);
  });

  it("a move clears every block in one undo step (all or nothing)", async () => {
    const m = new MockBackend();
    const id = m.mockFileTab("t.csv", [["h1", "h2", "h3", "h4"], ["1", "2", "3", "4"], ["5", "6", "7", "8"], ["9", "10", "11", "12"]]).id;
    const blocks = [r(1, 1, 0, 1), r(2, 1, 0, 1), r(3, 1, 0, 1)];
    await m.fillAreas(id, blocks, "");
    expect(await m.copyCells(id, 1, 3, 0, 2)).toBe("\t2\r\n\t6\r\n\t10\r\n");
    await m.change(id, { op: "undo" });
    expect(await m.copyCells(id, 1, 3, 0, 1)).toBe("1\r\n5\r\n9\r\n");
  });
});

describe("a drop on the other window", () => {
  const saved = MockBackend.DROP_DEADLINE_MS;
  afterEach(() => {
    MockBackend.DROP_DEADLINE_MS = saved;
    const g = globalThis as { __ndeDropFreeze?: boolean; __ndeDropDelay?: number };
    g.__ndeDropFreeze = false;
    g.__ndeDropDelay = 0;
  });

  /** Two windows: a source with made-up rows, a target that takes drops at row 1, col 0. */
  async function two() {
    const src = new MockBackend();
    const dst = new MockBackend();
    const s = src.mockFileTab("from.csv", [["a", "b"], ["Zed", "1"], ["Ya", "2"]]).id;
    const d = dst.mockFileTab("to.csv", [["x", "y"], ["", ""]]).id;
    const notices: DropNotice[] = [];
    dst.onDropCells((n) => {
      notices.push(n);
      void dst.dropPaste(n.doc, n.id, { row: 1, col: 0, op: "none", skip_blanks: false, transpose: false }).catch(() => {});
    });
    src.onSelectDoc(() => {});
    await dst.setTabStrip([{ doc: d, x: 0, y: 0, w: 100, h: 30 }]);
    await wait(30);
    const there = await src.otherWindowTabs();
    expect(there.map((t) => t.name)).toEqual(["to"]);
    return { src, dst, s, d, remote: there[0]!.doc, notices };
  }

  it("copies through the shell's stand-in, and the source hears it was done", async () => {
    const { src, dst, s, d, remote, notices } = await two();
    const id = await src.dropBegin(s, [r(1, 2, 0, 2)], [], remote, false);
    expect(await src.dropWait(id)).toMatchObject({ kind: "done", rows: 2, width: 2 });
    expect(notices[0]!.from_name).toBe("from");
    expect(JSON.stringify(notices[0])).not.toContain("Zed");
    expect(await dst.copyCells(d, 1, 2, 0, 2)).toBe("Zed\t1\r\nYa\t2\r\n");
  });

  it("a frozen target: too late, and nothing is written", async () => {
    MockBackend.DROP_DEADLINE_MS = 200;
    const { src, dst, s, d, remote } = await two();
    (globalThis as { __ndeDropFreeze?: boolean }).__ndeDropFreeze = true;
    const id = await src.dropBegin(s, [r(1, 2, 0, 2)], [], remote, true);
    expect(await src.dropWait(id)).toEqual({ kind: "too_late" });
    expect(await dst.copyCells(d, 1, 1, 0, 2)).toBe("\t\r\n");
  });

  it("a late target is refused and writes nothing; the source says too late", async () => {
    MockBackend.DROP_DEADLINE_MS = 150;
    const { src, dst, s, d, remote } = await two();
    (globalThis as { __ndeDropDelay?: number }).__ndeDropDelay = 300;
    const id = await src.dropBegin(s, [r(1, 2, 0, 2)], [], remote, false);
    expect(await src.dropWait(id)).toEqual({ kind: "too_late" });
    await wait(400);
    expect(await dst.copyCells(d, 1, 1, 0, 2)).toBe("\t\r\n");
  });

  it("private cells never go to a file that is not private (refused before the other window hears)", async () => {
    const { src, s, remote, notices } = await two();
    await src.setPrivate(s, false, true);
    await expect(src.dropBegin(s, [r(1, 2, 0, 2)], [], remote, false)).rejects.toMatchObject({ kind: "private" });
    await wait(30);
    expect(notices).toEqual([]);
  });
});
