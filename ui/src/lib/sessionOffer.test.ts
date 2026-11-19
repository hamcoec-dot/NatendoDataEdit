import { describe, expect, it } from "vitest";
import type { SessionItem } from "./backend";
import { sessionToOffer } from "./sessionOffer";

const item = (index: number, more: Partial<SessionItem> = {}): SessionItem => ({ index, name: `f${index}.csv`, folder: "Data", changed: false, missing: false, active: false, ...more });

describe("reopen last time's files", () => {
  it("offers nothing when the only file is the one just opened", () => {
    expect(sessionToOffer([item(0, { open: true, active: true })])).toEqual([]);
  });
  it("offers only the files not open and not gone", () => {
    const got = sessionToOffer([item(0, { open: true }), item(1), item(2, { missing: true }), item(3)]);
    expect(got.map((i) => i.index)).toEqual([1, 3]);
  });
});
