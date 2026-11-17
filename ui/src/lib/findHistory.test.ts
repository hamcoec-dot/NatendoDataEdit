import { describe, expect, it } from "vitest";
import { clearFindHistory, findHistory, rememberFind, stepHistory } from "./findHistory.svelte";

describe("find history", () => {
  it("keeps the last 20 searches, newest first, without repeats", () => {
    clearFindHistory();
    for (let i = 0; i < 25; i++) rememberFind(`s${i}`);
    rememberFind("s20");
    expect(findHistory.finds.length).toBe(20);
    expect(findHistory.finds[0]).toBe("s20");
    expect(findHistory.finds.filter((x) => x === "s20").length).toBe(1);
    rememberFind("");
    expect(findHistory.finds[0]).toBe("s20");
    clearFindHistory();
    expect(findHistory.finds).toEqual([]);
  });

  it("steps through with Up and Down", () => {
    const list = ["c", "b", "a"];
    expect(stepHistory(list, "typed", true)).toBe("c");
    expect(stepHistory(list, "c", true)).toBe("b");
    expect(stepHistory(list, "a", true)).toBeNull();
    expect(stepHistory(list, "b", false)).toBe("c");
    expect(stepHistory(list, "c", false)).toBe("");
    expect(stepHistory([], "x", true)).toBeNull();
  });
});
