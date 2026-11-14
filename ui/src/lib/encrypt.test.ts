import { describe, expect, it } from "vitest";
import { passwordProblem } from "./encrypt";

describe("encrypted files", () => {
  it("checks a new password", () => {
    expect(passwordProblem("short", "short")).toMatch(/at least 12/);
    expect(passwordProblem("a long password", "a long passwork")).toMatch(/not the same/);
    expect(passwordProblem("a long password", "a long password")).toBe("");
  });
});
