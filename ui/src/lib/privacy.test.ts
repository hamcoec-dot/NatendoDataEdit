import { describe, expect, it } from "vitest";
import type { Finding } from "./backend";
import { describe as describeStep, runStep, stepProblem, TOOLS } from "./clean";
import { MockBackend } from "./mock";
import { prefs, parsePrefs } from "./prefs.svelte";
import { copyTouches, findingsText } from "./privacy";

const fin = { format: null, part: 0, raw_dates: false, record: "" };
const f = (col: number, kind: Finding["kind"], title: string): Finding => ({ col, kind, title, looked: 1, hits: 1, by_title: true });

describe("private data (M13)", () => {
  it("names what was found and which copies touch it", () => {
    const list = [f(2, "ssn", "SSN"), f(5, "email", "Mail")];
    expect(findingsText(list)).toBe("SSN (Social Security numbers), Mail (e-mail addresses)");
    expect(findingsText([1, 2, 3, 4, 5, 6].map((c) => f(c, "phone", `P${c}`)))).toMatch(/and 2 more$/);
    expect(copyTouches(list, 0, 2)).toBe(false);
    expect(copyTouches(list, 1, 2)).toBe(true);
    expect(copyTouches(list, 5, 1)).toBe(true);
  });

  it("offers masking steps with checks", () => {
    for (const id of ["mask", "date_band", "scramble"]) expect(TOOLS.find((t) => t.id === id)?.group).toBe("Privacy");
    expect(runStep({ tool: "mask", keep_last: 4, with: "*", all: false }, "123-45-6789", 0)).toBe("***-**-6789");
    expect(runStep({ tool: "date_band", to: { kind: "year" } }, "1980-03-14", 0)).toBe("1980");
    expect(stepProblem({ tool: "scramble", phrase: "short" })).toMatch(/at least 16/);
    expect(stepProblem({ tool: "scramble", phrase: "fifteen chars!!" })).not.toBe("");
    expect(stepProblem({ tool: "scramble", phrase: "a long enough phrase" })).toBe("");
    expect(stepProblem({ tool: "mask", keep_last: 4, with: "**", all: false })).toMatch(/one character/);
    expect(stepProblem({ tool: "date_band", to: { kind: "age_band", size: 0, top: 90 } })).toMatch(/1 to 150/);
    expect(describeStep({ tool: "date_band", to: { kind: "age_band", size: 10, top: 90 } })).toBe("Date to age band (10 years, 90+)");
    const a = runStep({ tool: "scramble", phrase: "long enough" }, "x", 0);
    expect(a).toHaveLength(16);
    expect(runStep({ tool: "scramble", phrase: "long enough" }, "x", 0)).toBe(a);
  });

  it("warns by default and finds private columns in the stub backend", async () => {
    expect(parsePrefs(null).privacyWarnings).toBe(true);
    expect(parsePrefs('{"privacyWarnings":false}').privacyWarnings).toBe(false);
    expect(typeof prefs.data.privacyWarnings).toBe("boolean");
    const b = new MockBackend();
    const d = await b.importFinish((await b.importText("name,ssn,email\nAnn,123-45-6789,a@b.org\nBob,987-65-4321,c@d.net")).id, fin);
    const found = await b.findPrivate(d.id, true);
    expect(found.map((x) => [x.col, x.kind])).toEqual([
      [1, "ssn"],
      [2, "email"],
    ]);
    const rep = await b.saveSafeCopy(d.id, { columns: [{ action: "keep" }, { action: "drop" }], header: true, read: { decimal_comma: false, date_order: "mdy" } });
    expect(rep).toMatchObject({ rows: 2, cols: 2 });
    expect(await b.setPrivate(d.id, false, true)).toEqual({ file: true, folder: false });
    expect(await b.privateState(d.id)).toEqual({ file: true, folder: false });
  });
});
