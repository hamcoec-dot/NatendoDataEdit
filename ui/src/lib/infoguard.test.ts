import { describe, expect, it } from "vitest";
import { InfoGuard } from "./infoguard";
import type { DocInfo } from "./backend";

const info = (over: Partial<DocInfo>): DocInfo => ({ id: 1, name: "a.csv", rows: 10, history_pos: 3, data_revision: 5, ...over }) as DocInfo;

/** A guard whose answers come from a script, and whose waiting is instant. */
function scripted(answers: DocInfo[]) {
  let i = 0;
  const g = new InfoGuard(
    async () => answers[Math.min(i++, answers.length - 1)]!,
    async () => {},
  );
  return { g, calls: () => i };
}

describe("a stale answer never becomes the tab's state", () => {
  it("returns the last fresh info for a stale one, and tells the listener the fresh one that follows", async () => {
    const fresh = info({});
    const stale = info({ rows: 999, history_pos: 0, data_revision: 1, stale: true });
    const later = info({ rows: 11, history_pos: 4, data_revision: 6 });
    const { g } = scripted([fresh, stale, stale, later]);
    const told: DocInfo[] = [];
    g.onFresh = (i) => told.push(i);
    expect((await g.get(1)).rows).toBe(10);
    const got = await g.get(1);
    expect(got).toBe(fresh);
    expect(got.stale).toBeFalsy();
    await new Promise((r) => setTimeout(r, 20));
    expect(told).toEqual([later]);
    expect(told.every((t) => !t.stale)).toBe(true);
  });

  it("with no fresh answer yet it asks again until there is one (never returns a stale one)", async () => {
    const stale = info({ rows: 0, stale: true });
    const fresh = info({ rows: 7 });
    const { g, calls } = scripted([stale, stale, stale, fresh]);
    const got = await g.get(1);
    expect(got.rows).toBe(7);
    expect(calls()).toBe(4);
  });

  it("a stale answer with no job running is followed by a fresh one (the wait is at most 250 ms)", async () => {
    // WouldBlock once, nothing running: the retry delay is the guard's own constant.
    const { RETRY_MS } = await import("./infoguard");
    expect(RETRY_MS).toBeLessThanOrEqual(250);
    const fresh = info({});
    const { g } = scripted([fresh, info({ stale: true }), info({ data_revision: 6 })]);
    const told: DocInfo[] = [];
    g.onFresh = (i) => told.push(i);
    await g.get(1);
    await g.get(1);
    await new Promise((r) => setTimeout(r, 20));
    expect(told.length).toBe(1);
    expect(told[0]!.data_revision).toBe(6);
  });

  it("only one retry runs per document at a time", async () => {
    const fresh = info({});
    const stale = info({ stale: true });
    const { g, calls } = scripted([fresh, stale, stale, stale, fresh]);
    await g.get(1);
    await Promise.all([g.get(1), g.get(1), g.get(1)]);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls()).toBeLessThanOrEqual(7);
  });
});
