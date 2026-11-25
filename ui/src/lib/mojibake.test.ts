import { describe, expect, it } from "vitest";
import CASES from "../../../crates/engine/src/testdata/mojibake.json";
import { fixMojibake } from "./mojibake";

// The engine (encoding.rs) runs the same file.
describe("fix mojibake: the shared corpus", () => {
  for (const [text, want] of CASES.cases as [string, string | null][]) {
    it(`${JSON.stringify(text)} -> ${JSON.stringify(want)}`, () => {
      expect(fixMojibake(text)).toBe(want);
      if (want !== null) expect(fixMojibake(want)).toBeNull();
    });
  }
});
