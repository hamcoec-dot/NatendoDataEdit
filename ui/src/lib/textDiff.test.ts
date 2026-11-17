import { describe, expect, it } from "vitest";
import { cellDiff } from "./textDiff";

describe("cell diff", () => {
  it("finds the changed middle", () => {
    expect(cellDiff("555-1234", "555 1234")).toEqual({ head: "555", del: "-", ins: " ", tail: "1234" });
    expect(cellDiff("Main St", "Main Street")).toEqual({ head: "Main St", del: "", ins: "reet", tail: "" });
    expect(cellDiff("abc", "abc")).toEqual({ head: "abc", del: "", ins: "", tail: "" });
    expect(cellDiff("aa", "a")).toEqual({ head: "a", del: "a", ins: "", tail: "" });
    expect(cellDiff("José", "Jose")).toEqual({ head: "Jos", del: "é", ins: "e", tail: "" });
  });
});
