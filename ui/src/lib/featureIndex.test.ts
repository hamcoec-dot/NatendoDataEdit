import appSource from "../App.svelte?raw";
import { describe, expect, it } from "vitest";
import readme from "../../../README.md?raw";
import { sentences, bestSentence, bulletKey, clip, FEATURE_LINKS, featureDocs, plainMarkdown, readmeBullets } from "./featureIndex";
import { TOOLS } from "./clean";

/** Every command id App.svelte registers. */
function commandIds(): Set<string> {
  const app = appSource;
  const ids = new Set([...app.matchAll(/\bid: "([a-zA-Z0-9_-]+\.[a-zA-Z0-9._-]+)"/g)].map((m) => m[1]!));
  for (const t of TOOLS) ids.add(`clean.tool.${t.id}`);
  return ids;
}

describe("README feature index", () => {
  const bullets = readmeBullets(readme);

  it("reads the feature sections", () => {
    expect(bullets.length).toBeGreaterThan(60);
    const sections = new Set(bullets.map((b) => b.section));
    expect(sections).toContain("Cleaning tools");
    expect(sections).toContain("Privacy-safe defaults");
    expect(sections).not.toContain("Build from source");
    expect(bullets.every((b) => !b.text.includes("**"))).toBe(true);
  });

  it("links every README bullet to commands (add new bullets to FEATURE_LINKS)", () => {
    const missing = bullets.filter((b) => !(b.key in FEATURE_LINKS)).map((b) => `"${b.key}" (${b.section})`);
    expect(missing).toEqual([]);
  });

  it("has no links for bullets the README no longer has", () => {
    const keys = new Set(bullets.map((b) => b.key));
    expect(Object.keys(FEATURE_LINKS).filter((k) => !keys.has(k))).toEqual([]);
  });

  it("only links to commands that exist", () => {
    const ids = commandIds();
    const bad = Object.values(FEATURE_LINKS).flat().filter((id) => !ids.has(id));
    expect(bad).toEqual([]);
  });

  it("gives each linked command a short description from the README", () => {
    const docs = featureDocs(bullets, FEATURE_LINKS, (id) => id.split(".").pop()!);
    const d = docs.get("data.match")!;
    expect(d.section).toBe("Matching people and addresses");
    expect(d.text).toContain("nicknames");
    expect(d.blurb.length).toBeLessThanOrEqual(120);
    expect(docs.get("tools.watch")!.blurb).toMatch(/folder/);
    expect(docs.get("data.join")!.blurb).toMatch(/^Add new columns/);
    expect(docs.get("help.search")!.text).toBe("");
    expect(docs.get("help.search")!.blurb).toMatch(/Alt\+Q/);
  });

  it("does not cut descriptions at a semicolon or inside brackets", () => {
    const docs = featureDocs(bullets, FEATURE_LINKS, (id) => id.split(".").pop()!);
    for (const [id, d] of docs) {
      const open = (d.blurb.match(/\(/g) ?? []).length;
      const close = (d.blurb.match(/\)/g) ?? []).length;
      if (!d.blurb.endsWith("…")) expect(open, id).toBe(close);
    }
  });
});

describe("helpers", () => {
  it("turns markdown into plain text", () => {
    expect(plainMarkdown("**Bold:** `code` and [link](x) *it*")).toBe("Bold: code and link it");
  });
  it("keys bullets by bold title or first words", () => {
    expect(bulletKey("**Share-safe copy** (File > Save)")).toBe("share safe copy");
    expect(bulletKey("Find and replace with plain text, wildcards")).toBe("find and replace with plain");
  });
  it("clips at a word break", () => {
    expect(clip("one two three four", 10)).toBe("one two…");
    expect(clip("short", 10)).toBe("short");
  });
  it("splits sentences outside brackets only", () => {
    expect(sentences("One (a. b) two; three. Four.")).toEqual(["One (a. b) two; three.", "Four."]);
  });
  it("picks the sentence naming the command", () => {
    expect(bestSentence("Save all things. Revert to saved copy.", "Revert to saved")).toBe("Revert to saved copy.");
  });
});
