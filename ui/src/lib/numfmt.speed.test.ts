// How long showing a million mixed cells takes: the number format (as on main)
// and a custom code with red negatives, a time and a date and time. Best of 5.
// Off in the normal run; NDE_BENCH=1 npx vitest run src/lib/numfmt.speed.test.ts
// prints one JSON line (the same file runs on main, where only "number" exists).
import { expect, it } from "vitest";
import { defaultFormat, formatCell, type DisplayFormat } from "./m9";

const env = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env ?? {};
it.skipIf(!env.NDE_BENCH)("1,000,000 mixed cells", () => {
  const cells: string[] = [];
  let x = 0x9e3779b9;
  for (let i = 0; i < 1_000_000; i++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    const v = (x >>> 0) % 1_000_000;
    cells.push(i % 4 === 0 ? `-${v}.${v % 100}` : i % 4 === 1 ? `${v}.5` : i % 4 === 2 ? `2026-03-${(v % 28) + 1} ${v % 24}:${String(v % 60).padStart(2, "0")}` : `name ${v}`);
  }
  const base = defaultFormat("number") as DisplayFormat & Record<string, unknown>;
  const fmts: [string, DisplayFormat][] = [
    ["number", base],
    ["custom", { ...base, kind: "custom", code: "#,##0.00;[Red]-#,##0.00" } as DisplayFormat],
    ["time", { ...base, kind: "time", time: "hm12" } as DisplayFormat],
    ["datetime", { ...base, kind: "datetime", date: "medium", time: "hm12" } as DisplayFormat],
  ];
  const out: Record<string, number> = {};
  for (const [name, f] of fmts) {
    let best = Infinity;
    for (let run = 0; run < 5; run++) {
      const t0 = performance.now();
      let n = 0;
      for (const c of cells) n += formatCell(c, f).length;
      best = Math.min(best, performance.now() - t0);
      expect(n).toBeGreaterThan(0);
    }
    out[name] = Math.round(best * 10) / 10;
  }
  console.log(`NDE_BENCH ${JSON.stringify(out)}`);
}, 600_000);
