import { afterEach, describe, expect, it } from "vitest";
import { batchSummary, type BatchStatus } from "./batch";
import { answerPairs, keepAnswers, lastAnswers, questionNameOk } from "./recipe";

describe("recipe questions", () => {
  it("takes names as the engine does", () => {
    expect(questionNameOk("county")).toBe(true);
    expect(questionNameOk("Election date_2")).toBe(true);
    expect(questionNameOk("  ")).toBe(false);
    expect(questionNameOk("a{b}")).toBe(false);
    expect(questionNameOk("x".repeat(41))).toBe(false);
  });

  it("sends each question's answer, or its default", () => {
    const qs = [
      { name: "county", prompt: "Which county?", default: "Knox" },
      { name: "year", prompt: "", default: "2026" },
    ];
    expect(answerPairs(qs, { county: "Hamilton" })).toEqual([
      ["county", "Hamilton"],
      ["year", "2026"],
    ]);
  });

  describe("remembered answers", () => {
    const store = new Map<string, string>();
    const g = globalThis as { localStorage?: unknown };
    afterEach(() => {
      store.clear();
      delete g.localStorage;
    });
    it("keeps the last answers of the 50 recipes run last", () => {
      g.localStorage = {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => void store.set(k, v),
      };
      keepAnswers("monthly.json", { county: "Knox" });
      expect(lastAnswers("monthly.json")).toEqual({ county: "Knox" });
      for (let i = 0; i < 60; i++) keepAnswers(`r${i}.json`, { n: String(i) });
      expect(lastAnswers("monthly.json")).toEqual({});
      expect(lastAnswers("r59.json")).toEqual({ n: "59" });
    });
    it("works without storage", () => {
      keepAnswers("x.json", { a: "1" });
      expect(lastAnswers("x.json")).toEqual({});
    });
  });
});

describe("run recipe on files", () => {
  const st = (results: BatchStatus["results"]): BatchStatus => ({
    files: [],
    recipe: null,
    steps: [],
    questions: [],
    out: null,
    running: false,
    done: results.filter((r) => r.ok).length,
    failed: results.filter((r) => !r.ok).length,
    current: null,
    results,
  });
  it("sums rows over the files done", () => {
    const s = st([
      { name: "a.csv", ok: true, rows_in: 1200, rows_out: 1000, message: "" },
      { name: "b.csv", ok: true, rows_in: 10, rows_out: 5, message: "" },
      { name: "c.csv", ok: false, rows_in: 0, rows_out: 0, message: "its columns differ" },
    ]);
    expect(batchSummary(s)).toBe(`2 files done; 1 file not done; ${(1210).toLocaleString()} rows in, ${(1005).toLocaleString()} rows out.`);
    expect(batchSummary(st([]))).toBe("0 files done.");
  });
});
