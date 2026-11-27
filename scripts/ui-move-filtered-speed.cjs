// Speed of moving rows held down (gap #5), in the browser preview (Edge, the
// stub's 1M x 7 sample, CPU slowed 20x). Alt+Shift+Down pressed 20 times on one
// row; the draw times after each press. Cases: no filter; with a filter on
// (main refuses filtered moves, so compare with its no-filter case). Then, with
// no slowdown, RowHeights.moveRuns over 100,000 heights.
// Run: (cd ui && npx vite --port 5381) then
// NODE_PATH=<folder with playwright> node scripts/ui-move-filtered-speed.cjs [url]
// Prints one JSON line: draw median and p95 per case (best of 3).
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5381";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  const cdp = await ctx.newCDPSession(p);
  const pick = (d) => {
    d.sort((a, c) => a - c);
    return d.length ? { median: +d[Math.floor(d.length / 2)].toFixed(2), p95: +d[Math.floor(d.length * 0.95)].toFixed(2), n: d.length } : { median: 0, p95: 0, n: 0 };
  };
  const held = async () => {
    const runs = [];
    for (let k = 0; k < 3; k++) {
      await p.evaluate(() => {
        window.__nde.tab.scrollY = 0;
        window.__nde.grid.select({ row: 3, col: 0 });
        window.__nde.grid.focus();
      });
      await p.waitForTimeout(400);
      await p.evaluate(() => (window.__ndeDrawLog = []));
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
      for (let i = 0; i < 20; i++) {
        await p.keyboard.press("Alt+Shift+ArrowDown");
        await p.waitForTimeout(30);
      }
      await p.waitForTimeout(600);
      const d = await p.evaluate(() => {
        const d = window.__ndeDrawLog;
        window.__ndeDrawLog = undefined;
        return d;
      });
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
      runs.push(pick(d));
      // Put the rows back (20 Undos).
      for (let i = 0; i < 20; i++) await p.evaluate(() => window.__nde.reg.run("edit.undo"));
      await p.waitForTimeout(400);
    }
    runs.sort((a, c) => a.median - c.median);
    return runs[0];
  };
  const spec = { columns: [{ col: 3, rule: { type: "values", values: ["Osaka", "Perth", "Lisbon"], blanks: false } }], keep_first: true };
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
  await p.waitForTimeout(1500);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  const res = {};
  res.unfiltered = await held();
  await p.evaluate(async (spec) => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setFilter(t.id, spec), false);
    t.bump();
  }, spec);
  await p.waitForTimeout(800);
  res.filtered = await held();
  res.moveRuns_100k_ms = await p.evaluate(async () => {
    const { RowHeights } = await import("/src/lib/rowheights.ts");
    if (!RowHeights.prototype.moveRuns) return null;
    const mk = () => {
      const h = new RowHeights();
      h.set(Array.from({ length: 100000 }, (_, i) => 1 + i * 10), 40);
      return h;
    };
    const runs = Array.from({ length: 1000 }, (_, i) => [5 + i * 700, 3]);
    const t = [];
    for (let k = 0; k < 9; k++) {
      const h = mk();
      const a = performance.now();
      h.moveRuns(runs, 400000);
      t.push(performance.now() - a);
    }
    t.sort((x, y) => x - y);
    return +t[4].toFixed(2);
  });
  console.log(JSON.stringify(res));
  await b.close();
})();
