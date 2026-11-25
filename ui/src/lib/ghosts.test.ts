import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GhostLoader, GHOST_DEBOUNCE_MS, clearSkipsWhenClosed, previewMark, previewStale, visibleGhost, type GhostMap } from "./ghosts";
import { MockBackend } from "./mock";
import type { FindSpec, GhostWindow } from "./backend";

const win = (cells: [number, number, string][], partial = false): GhostWindow => ({
  ghosts: cells.map(([row, col, after]) => ({ row, number: row, col, after, cut: false })),
  partial,
});

describe("the ghost loader", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup() {
    let shown: GhostMap = new Map();
    const sets: GhostMap[] = [];
    const loader = new GhostLoader({
      has: () => shown.size > 0,
      set: (m) => {
        shown = m;
        sets.push(m);
      },
    });
    return { loader, shown: () => shown, sets };
  }

  it("types fast against a slow answer: only the last key's ghosts are painted", async () => {
    const { loader, shown } = setup();
    const resolvers: ((w: GhostWindow[]) => void)[] = [];
    const ask = (tag: string) => () => new Promise<GhostWindow[]>((r) => resolvers.push((w) => r(w.map((x) => ({ ...x, ghosts: x.ghosts.map((g) => ({ ...g, after: tag })) })))));
    // Five keystrokes 20 ms apart: one ask, for the last one.
    for (const k of ["a", "ab", "abc", "abcd", "abcde"]) {
      loader.want(`base-${k}`, `key-${k}`, ask(k));
      await vi.advanceTimersByTimeAsync(20);
    }
    expect(resolvers.length).toBe(0);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS);
    expect(resolvers.length).toBe(1);
    resolvers[0]!([win([[1, 0, "x"]])]);
    await vi.advanceTimersByTimeAsync(1);
    expect(shown().get("1:0")?.after).toBe("abcde");
  });

  it("an answer that comes after a newer key is dropped, never drawn", async () => {
    const { loader, shown, sets } = setup();
    let slow!: (w: GhostWindow[]) => void;
    loader.want("b1", "k1", () => new Promise((r) => (slow = r)));
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    loader.want("b2", "k2", async () => [win([[2, 0, "new"]])]);
    slow([win([[1, 0, "old"]])]);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect([...shown().keys()]).toEqual(["2:0"]);
    expect(sets.every((m) => !m.has("1:0"))).toBe(true);
  });

  it("a new search, replacement or revision clears what is drawn at once; a scroll keeps it until the window answers", async () => {
    const { loader, shown } = setup();
    loader.want("b1", "k1", async () => [win([[1, 0, "x"]])]);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect(shown().size).toBe(1);
    // scrolling: same base, new window
    loader.want("b1", "k1-scrolled", async () => [win([[50, 0, "y"]])]);
    expect(shown().size).toBe(1);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect([...shown().keys()]).toEqual(["50:0"]);
    // an edit moves the revision: the old ghosts go before anything new arrives
    loader.want("b2", "k2", async () => [win([[3, 0, "z"]])]);
    expect(shown().size).toBe(0);
  });

  it("off() drops the ghosts and any ask still waiting", async () => {
    const { loader, shown } = setup();
    const ask = vi.fn(async () => [win([[1, 0, "x"]])]);
    loader.want("b", "k", ask);
    loader.off();
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect(ask).not.toHaveBeenCalled();
    expect(shown().size).toBe(0);
  });

  it("a failed ask draws nothing and is asked again on the next draw", async () => {
    const { loader, shown } = setup();
    let n = 0;
    const ask = async () => {
      if (n++ === 0) throw new Error("busy");
      return [win([[1, 0, "x"]])];
    };
    loader.want("b", "k", ask);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect(shown().size).toBe(0);
    loader.want("b", "k", ask);
    await vi.advanceTimersByTimeAsync(GHOST_DEBOUNCE_MS + 1);
    expect(shown().size).toBe(1);
  });
});

describe("unticked changes follow their row, not its place", () => {
  it("a change unticked in the preview hides the ghost of that row's number, whatever row it is shown on now", () => {
    // Row number 7 was shown on row 3 when it was unticked; after a sort it is shown on row 9.
    const ghosts: GhostMap = new Map([["9:2", { number: 7, after: "x", cut: false }], ["3:2", { number: 12, after: "y", cut: false }]]);
    const skip = new Set(["7:2"]);
    expect(visibleGhost(ghosts, skip, 9, 2)).toBeUndefined();
    expect(visibleGhost(ghosts, skip, 3, 2)?.after).toBe("y");
    // another column of the same row is not unticked
    expect(visibleGhost(new Map([["9:3", { number: 7, after: "z", cut: false }]]), skip, 9, 3)?.after).toBe("z");
  });
});

const TAB = String.fromCharCode(9);
const CRLF = String.fromCharCode(13, 10);

describe("the preview's unticked changes live and die with the preview", () => {
  it("closing the preview, however it closes, clears them (one helper)", () => {
    const skip = new Set(["7:2", "9:2"]);
    expect(clearSkipsWhenClosed(true, skip)).toBe(false);
    expect(skip.size).toBe(2);
    expect(clearSkipsWhenClosed(false, skip)).toBe(true);
    expect(skip.size).toBe(0);
    expect(clearSkipsWhenClosed(false, skip)).toBe(false);
  });

  it("an edit, a sort or an undo while the preview is open makes it stale; a filter does not", async () => {
    const m = new MockBackend();
    const d = await m.newDocument();
    await m.dataOp(d.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: ["b", "a", "c", "a"].join(String.fromCharCode(13, 10)) });
    const open = previewMark(await m.docInfo(d.id));
    expect(previewStale(open, await m.docInfo(d.id))).toBe(false);
    await m.sortRows(d.id, { keys: [{ col: 0, descending: false, kind: "text", match_case: false }], keep_first: false });
    expect(previewStale(open, await m.docInfo(d.id))).toBe(true);
    const again = previewMark(await m.docInfo(d.id));
    await m.setCells(d.id, [[0, 0, "z"]]);
    expect(previewStale(again, await m.docInfo(d.id))).toBe(true);
    expect(previewStale(null, await m.docInfo(d.id))).toBe(false);
  });
});

describe("the browser stand-in's ghosts and count", () => {
  const spec = (text: string, extra: Partial<FindSpec> = {}): FindSpec =>
    ({ pattern: { text, kind: "plain", match_case: false, whole_cell: false }, scope: { kind: "all" }, skip_first: false, skip_cols: [], hidden_cols: [], ...extra }) as FindSpec;

  async function table(text: string) {
    const m = new MockBackend();
    const d = await m.newDocument();
    await m.dataOp(d.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text });
    return { m, doc: d.id };
  }

  it("shows the cells Replace all changes, and the count is the whole table", async () => {
    const { m, doc } = await table(["ab,b,ab", "x,ab,y", "ab,,ab"].map((r) => r.split(",").join(TAB)).join(CRLF));
    const g = await m.replaceGhosts(doc, spec("ab"), "Z", 0, 2, 0, 3);
    expect(g.ghosts.map((x) => [x.row, x.col, x.after])).toEqual([[0, 0, "Z"], [0, 2, "Z"], [1, 1, "Z"]]);
    expect(await m.replaceCount(doc, spec("ab"), "Z")).toBe(5);
  });

  it("with several blocks selected, ghosts and the count use the blocks (the areas scope)", async () => {
    const { m, doc } = await table(["ab,ab,ab", "ab,ab,ab", "ab,ab,ab"].map((r) => r.split(",").join(TAB)).join(CRLF));
    const areas = [{ row: 0, rows: 1, col: 0, cols: 1 }, { row: 2, rows: 1, col: 1, cols: 2 }];
    const s = spec("ab", { scope: { kind: "areas", areas } } as Partial<FindSpec>);
    const g = await m.replaceGhosts(doc, s, "Z", 0, 3, 0, 3);
    expect(g.ghosts.map((x) => [x.row, x.col])).toEqual([[0, 0], [2, 1], [2, 2]]);
    expect(await m.replaceCount(doc, s, "Z")).toBe(3);
  });

  it("leaves out hidden columns, and a newer count cancels the older", async () => {
    const { m, doc } = await table("ab\tab\r\nab\tab");
    const g = await m.replaceGhosts(doc, spec("ab", { hidden_cols: [1] } as Partial<FindSpec>), "Z", 0, 2, 0, 2);
    expect(g.ghosts.every((x) => x.col === 0)).toBe(true);
    let open!: () => void;
    m.countGate = new Promise<void>((r) => (open = r));
    const first = m.replaceCount(doc, spec("ab"), "Z").then(() => "done", (e) => `cancelled:${e.kind ?? "?"}`);
    const second = m.replaceCount(doc, spec("ab"), "Q");
    open();
    expect(await first).toMatch(/^cancelled/);
    expect(await second).toBe(4);
  });
});
