import { describe, expect, it } from "vitest";
import { noteSummary } from "./notes";

describe("notes and flags", () => {
  it("summarises a row's note", () => {
    expect(noteSummary(undefined)).toBe("");
    expect(noteSummary({ flag: 1, text: "call back" })).toBe("Red flag: call back");
    expect(noteSummary({ flag: 0, text: "x" })).toBe("x");
    expect(noteSummary({ flag: 4, text: "" })).toBe("Green flag");
  });
});
