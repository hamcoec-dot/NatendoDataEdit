import { describe, expect, it } from "vitest";
import { Tab } from "./tab.svelte";
import { InfoGuard } from "./infoguard";
import type { DocInfo } from "./backend";

const info = (pos: number, seq: number, over: Partial<DocInfo> = {}): DocInfo =>
  ({ id: 1, rows: 10, cols: 5, total_rows: 10, history_pos: pos, has_filter: false, filtered: false, seq, ...over }) as unknown as DocInfo;

describe("what the tab takes as its state", () => {
  it("a stale answer changes nothing (it could read as an Undo)", () => {
    const t = new Tab(info(0, 1), 100);
    t.update(info(1, 2), true);
    t.noteColMove(1, 1, 3);
    const moves = () => (t as unknown as { colMoves: unknown[] }).colMoves.length;
    expect(moves()).toBe(1);
    // A poll made while a change held the document: figures from before the move.
    t.update(info(0, 1, { stale: true }), false);
    expect(t.info.history_pos).toBe(1);
    expect(moves()).toBe(1);
    // (The same figures taken as a real answer would read as an Undo and take the move back.)
    t.update(info(0, 1), false);
    expect(moves()).toBe(1); // dropped too: older than what the tab holds
  });

  it("an edit, then a stale poll: the tab keeps the edit", () => {
    const t = new Tab(info(0, 1, { rows: 10 }), 100);
    t.update(info(1, 5, { rows: 12 }), true);
    t.update(info(1, 3, { rows: 10, stale: true }), false);
    expect(t.info.rows).toBe(12);
  });

  it("a reply older than what the tab holds is dropped, but the cells are still read again when the data changed", () => {
    const t = new Tab(info(0, 1), 100);
    t.update(info(2, 9, { rows: 12 }), true);
    let invalidated = 0;
    const orig = t.cache.invalidate.bind(t.cache);
    t.cache.invalidate = () => {
      invalidated++;
      orig();
    };
    t.update(info(1, 7, { rows: 11 }), true);
    expect(t.info.history_pos).toBe(2);
    expect(t.info.rows).toBe(12);
    expect(invalidated).toBe(1);
  });

  it("the retry's fresh answer landing after a newer edit reply does not replace it", async () => {
    // The guard asks again after a stale answer; meanwhile an operation's reply (newer) reaches the tab.
    let n = 0;
    const answers = [info(0, 1), info(0, 1, { stale: true }), info(0, 2)];
    const g = new InfoGuard(async () => answers[Math.min(n++, 2)]!, async () => {});
    const t = new Tab(info(0, 1), 100);
    g.onFresh = (i) => t.update(i, false);
    t.update(await g.get(1), false);
    t.update(await g.get(1), false); // stale: the guard hands back the last fresh one
    t.update(info(1, 10, { rows: 15 }), true); // an edit's reply arrives (newer than the retry's answer)
    await new Promise((r) => setTimeout(r, 20));
    expect(t.info.history_pos).toBe(1);
    expect(t.info.rows).toBe(15);
  });

  it("infos without a reading number (the browser stand-in) are taken as before", () => {
    const t = new Tab(info(0, 1), 100);
    t.update({ ...info(1, 0), seq: undefined } as DocInfo, true);
    expect(t.info.history_pos).toBe(1);
  });
});
