import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { HEADER_MEMORY_MAX, forgetHeaders, pathKey, recallHeader, recallColumnTypes, recallPad, rememberColumnTypes, rememberedHeaders, rememberHeader, rememberPad } from "./headerMemory";

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
    expect(rememberedHeaders()).toBe(HEADER_MEMORY_MAX);
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

  it("salts the fingerprint, so a guessed path can't be checked", async () => {
    const k1 = await pathKey("C:\\x.csv");
    expect(store.get("nde.headers.salt")).toMatch(/^[0-9a-f]{32}$/);
    forgetHeaders();
    expect(await pathKey("C:\\x.csv")).not.toBe(k1);
  });

  it("keeps the zero-padding answer per file as a fingerprint and yes/no only, and forgets it with the rest", async () => {
    expect(await recallPad("C:/Data/ids.xlsx")).toBeNull();
    await rememberPad("C:/Data/ids.xlsx", true);
    expect(await recallPad("c:/data/IDS.xlsx")).toBe(true);
    await rememberPad("C:/Data/ids.xlsx", false);
    expect(await recallPad("C:/Data/ids.xlsx")).toBe(false);
    const kept = store.get("nde.padzeros.v1") ?? "";
    expect(kept).not.toMatch(/ids|Data|xlsx/i);
    forgetHeaders();
    expect(store.has("nde.padzeros.v1")).toBe(false);
  });

  it("remembers a file's column types as a fingerprint and the types only, and never for a private file", async () => {
    expect(await recallColumnTypes("C:/Data/voters.csv")).toBeNull();
    await rememberColumnTypes("C:/Data/voters.csv", { cols: 4, types: ["id", "auto", "number"] });
    expect(await recallColumnTypes("c:/data/VOTERS.csv")).toEqual({ cols: 4, types: ["id", "auto", "number"] });
    const kept = store.get("nde.coltypes.v1") ?? "";
    expect(kept).not.toMatch(/voters|Data|csv/i);
    // All Auto: nothing to keep. A private file: the earlier entry is dropped and nothing is written.
    await rememberColumnTypes("C:/Data/voters.csv", { cols: 4, types: ["auto", "auto"] });
    expect(await recallColumnTypes("C:/Data/voters.csv")).toBeNull();
    await rememberColumnTypes("C:/Data/voters.csv", { cols: 4, types: ["id"] });
    await rememberColumnTypes("C:/Data/voters.csv", { cols: 4, types: ["id"] }, true);
    expect(await recallColumnTypes("C:/Data/voters.csv")).toBeNull();
    // forgetting everything clears them too
    await rememberColumnTypes("C:/Data/a.csv", { cols: 2, types: ["text"] });
    forgetHeaders();
    expect(store.has("nde.coltypes.v1")).toBe(false);
  });

  it("survives a damaged store", async () => {
    store.set("nde.headers.v1", "not json");
    expect(await recallHeader("C:\\x.csv")).toBeNull();
    await rememberHeader("C:\\x.csv", true);
    expect(await recallHeader("C:\\x.csv")).toBe(true);
  });
});
