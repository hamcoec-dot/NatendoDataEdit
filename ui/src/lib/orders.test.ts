import { describe, expect, it } from "vitest";
import { fillOrders, MAX_ORDERS, newOrderId, orderFromCells, parseList, readOrders } from "./m10";

describe("saved custom orders", () => {
  it("gives orders saved before ids an id each, in their order, and keeps ids that are there", () => {
    const got = readOrders([
      { name: "Priority", items: ["Urgent", "Normal"] },
      { id: "o9", name: "Sizes", items: ["S", "M", "L"] },
      { name: "Teams", items: ["Red", "Blue"] },
    ]);
    expect(got.map((o) => o.name)).toEqual(["Priority", "Sizes", "Teams"]);
    expect(got[1]!.id).toBe("o9");
    const ids = got.map((o) => o.id);
    expect(new Set(ids).size).toBe(3);
    expect(ids.every((i) => i.length > 0)).toBe(true);
    // Read again (as after a restart): the same ids.
    expect(readOrders(got).map((o) => o.id)).toEqual(ids);
  });

  it("drops bad entries, a repeated id, and keeps the limits", () => {
    const got = readOrders([null, 3, { name: "x" }, { id: "a", name: "A", items: ["1", 2, "3"] }, { id: "a", name: "B", items: ["z"] }]);
    expect(got.map((o) => [o.name, o.items])).toEqual([
      ["A", ["1", "3"]],
      ["B", ["z"]],
    ]);
    expect(got[0]!.id).toBe("a");
    expect(got[1]!.id).not.toBe("a");
    expect(readOrders(Array.from({ length: 80 }, (_, i) => ({ name: `n${i}`, items: ["v"] })))).toHaveLength(MAX_ORDERS);
    expect(readOrders("no")).toEqual([]);
  });

  it("keeps where an order came from", () => {
    expect(readOrders([{ name: "Names", items: ["a"], fromCells: true }])[0]!.fromCells).toBe(true);
    expect(readOrders([{ name: "Typed", items: ["a"] }])[0]!.fromCells).toBeUndefined();
  });

  it("makes new ids unlike the ones taken", () => {
    const taken = [{ id: "o2" }, { id: "o3" }];
    const id = newOrderId(taken);
    expect(taken.some((t) => t.id === id)).toBe(false);
    expect(newOrderId([...taken, { id }])).not.toBe(id);
  });
});

describe("an order from the selected cells", () => {
  it("reads row by row, left to right, trimmed, each value once", () => {
    expect(orderFromCells([["Red", " Blue"], ["", "red"], ["Green, light", "Blue"]])).toEqual({ items: ["Red", "Blue", "Green, light"], tooMany: false });
  });

  it("refuses more than 10,000 different values", () => {
    const rows = Array.from({ length: 10_001 }, (_, i) => [`v${i}`]);
    expect(orderFromCells(rows)).toEqual({ items: [], tooMany: true });
    expect(orderFromCells(rows.slice(0, 10_000)).items).toHaveLength(10_000);
  });

  it("is edited one value per line, so a value may hold a comma", () => {
    expect(parseList("Green, light\nBlue", false)).toEqual(["Green, light", "Blue"]);
    expect(parseList("Green, light\nBlue")).toEqual(["Green", "light", "Blue"]);
  });
});

describe("the fill handle and custom orders", () => {
  it("is on to start with for typed orders of up to 100 values, off for orders from cells and longer ones", () => {
    const hundred = Array.from({ length: 100 }, (_, i) => `v${i}`);
    const got = readOrders([
      { name: "Typed", items: ["Urgent", "Normal"] },
      { name: "Names", items: ["Ada Quill", "Bo Frost"], fromCells: true },
      { name: "Hundred", items: hundred },
      { name: "Long", items: [...hundred, "one more"] },
      { name: "Unticked", items: ["a", "b"], fill: false },
    ]);
    expect(got.map((o) => o.fill)).toEqual([true, false, true, false, false]);
    // One cell follows only ticked orders of up to 100 values, even when a long one is ticked.
    const long = { ...got[3]!, fill: true };
    expect(fillOrders([...got.slice(0, 3), long]).map((o) => [o.name, o.one_cell])).toEqual([
      ["Typed", true],
      ["Names", false],
      ["Hundred", true],
      ["Long", false],
      ["Low, Medium, High", false],
    ]);
  });

  it("keeps the Manage panel's order, the built-in Low, Medium, High last", () => {
    const got = fillOrders(readOrders([{ name: "B", items: ["x"] }, { name: "A", items: ["y"] }]));
    expect(got.map((o) => o.name)).toEqual(["B", "A", "Low, Medium, High"]);
  });
});
