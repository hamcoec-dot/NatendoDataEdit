import { describe, expect, it } from "vitest";
import { savedSearches } from "./prefs.svelte";

describe("saved searches", () => {
  it("keeps only well-formed, bounded entries", () => {
    const got = savedSearches([
      { name: "TN codes", text: "TN", replacement: "Tennessee", kind: "plain", matchCase: true },
      { name: "x".repeat(500), text: 5, kind: "evil", blank: "yes" },
      null,
      "nope",
      { text: "no name" },
    ]);
    expect(got.length).toBe(2);
    expect(got[0]).toMatchObject({ name: "TN codes", text: "TN", replacement: "Tennessee", kind: "plain", matchCase: true, blank: false });
    expect(got[1]!.name.length).toBe(100);
    expect(got[1]!.kind).toBe("plain");
    expect(got[1]!.text).toBe("");
    expect(savedSearches("not a list")).toEqual([]);
    expect(savedSearches(Array.from({ length: 300 }, (_, i) => ({ name: `s${i}` }))).length).toBe(100);
  });
});
