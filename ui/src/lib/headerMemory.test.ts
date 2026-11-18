import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { HEADER_MEMORY_MAX, forgetHeaders, pathKey, recallHeader, rememberHeader } from "./headerMemory";

describe("first row is headers, remembered per file", () => {
  const store = new Map<string, string>();
  const g = globalThis as { localStorage?: unknown };
  beforeEach(() => {
    g.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    };
  });
  afterEach(() => {
    store.clear();
    delete g.localStorage;
  });

  it("keeps the choice for the same file, whatever the case or slashes", async () => {
    expect(await recallHeader("C:\\Data\\voters.csv")).toBeNull();
    await rememberHeader("C:\\Data\\voters.csv", true);
    expect(await recallHeader("c:/data/VOTERS.csv")).toBe(true);
    await rememberHeader("C:\\Data\\voters.csv", false);
    expect(await recallHeader("C:\\Data\\voters.csv")).toBe(false);
    expect(await recallHeader("C:\\Data\\other.csv")).toBeNull();
  });

  it("keeps a fingerprint, never the path", async () => {
    await rememberHeader("C:\\Users\\pat\\Secret plans.csv", true);
    const kept = [...store.values()].join("");
    expect(kept).not.toMatch(/pat|Secret|plans|csv/i);
    expect(kept).toContain(await pathKey("C:\\Users\\pat\\Secret plans.csv"));
  });

  it("keeps nothing for a private file and forgets what it had", async () => {
    await rememberHeader("C:\\x.csv", true, true);
    expect(await recallHeader("C:\\x.csv")).toBeNull();
    await rememberHeader("C:\\x.csv", true);
    await rememberHeader("C:\\x.csv", true, true);
    expect(await recallHeader("C:\\x.csv")).toBeNull();
  });

  it("forgets the files used longest ago past the limit", async () => {
    for (let i = 0; i <= HEADER_MEMORY_MAX; i++) await rememberHeader(`C:\\f${i}.csv`, true);
    expect(await recallHeader("C:\\f0.csv")).toBeNull();
    expect(await recallHeader(`C:\\f${HEADER_MEMORY_MAX}.csv`)).toBe(true);
    // Speed: with the store full, a save or an open adds well under a frame.
    const t0 = performance.now();
    for (let i = 0; i < 20; i++) {
      await rememberHeader(`C:\\g${i}.csv`, false);
      await recallHeader(`C:\\g${i}.csv`);
    }
    expect((performance.now() - t0) / 20).toBeLessThan(15);
    forgetHeaders();
    expect(await recallHeader(`C:\\f${HEADER_MEMORY_MAX}.csv`)).toBeNull();
  });

  it("survives a damaged store", async () => {
    store.set("nde.headers.v1", "not json");
    expect(await recallHeader("C:\\x.csv")).toBeNull();
    await rememberHeader("C:\\x.csv", true);
    expect(await recallHeader("C:\\x.csv")).toBe(true);
  });
});
