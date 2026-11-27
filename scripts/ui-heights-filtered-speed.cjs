// Speed of the grid with custom row heights while filtered (gap #4), in the
// browser preview (Edge, the stub's 1M x 7 sample, CPU slowed 20x so the work
// per frame decides it). Cases: wheel scrolling with a filter on and no
// heights; with a filter on and 100,000 heights; without a filter and 100,000
// heights (main draws filtered rows plain, so this is its comparison); and,
// with heights, a filter changed 10 times in a row (calls to the engine).
// Run: (cd ui && npx vite --port 5398) then
// NODE_PATH=<folder with playwright> node scripts/ui-heights-filtered-speed.cjs [url]
// Prints one JSON line: draw median and p95 per case (best of 3).
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5398";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  const cdp = await ctx.newCDPSession(p);
  const pick = (d) => {
    d.sort((a, c) => a - c);
    return d.length ? { median: +d[Math.floor(d.length / 2)].toFixed(2), p95: +d[Math.floor(d.length * 0.95)].toFixed(2) } : { median: 0, p95: 0 };
  };
  const wheel = async () => {
    const cv = await (await p.$("canvas")).boundingBox();
    const runs = [];
    for (let k = 0; k < 3; k++) {
      await p.evaluate(() => {
        window.__nde.tab.scrollY = 0;
        window.__nde.grid.redraw();
      });
      await p.waitForTimeout(300);
      await p.evaluate(() => (window.__ndeDrawLog = []));
      await p.mouse.move(cv.x + 300, cv.y + 200);
      for (let i = 0; i < 60; i++) {
        await p.mouse.wheel(0, 240);
        await p.waitForTimeout(16);
      }
      const d = await p.evaluate(() => {
        const d = window.__ndeDrawLog;
        window.__ndeDrawLog = undefined;
        return d;
      });
      runs.push(pick(d));
    }
    runs.sort((a, c) => a.median - c.median);
    return runs[0];
  };
  const spec = { columns: [{ col: 3, rule: { type: "values", values: ["Osaka", "Perth", "Lisbon"], blanks: false } }], keep_first: true };
  const filter = (on) =>
    p.evaluate(async ([on, spec]) => {
      const t = window.__nde.tab;
      const be = window.__nde.backend;
      t.update(on ? await be.setFilter(t.id, spec) : await be.clearFilter(t.id), false);
      t.bump();
      await new Promise((r) => setTimeout(r, 600));
    }, [on, spec]);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
  await p.waitForTimeout(1500);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  const res = {};
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
  await filter(true);
  res.filtered_no_heights = await wheel();
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.rowHeights.set(Array.from({ length: 100_000 }, (_, i) => 1 + i * 10), 40);
    t.bump();
  });
  await p.waitForTimeout(2000);
  res.filtered_100k_heights = await wheel();
  await filter(false);
  res.unfiltered_100k_heights = await wheel();
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  // A filter changed 10 times in a row with heights (as typing would): engine calls.
  res.burst_calls = await p.evaluate(async (spec) => {
    const t = window.__nde.tab;
    const be = window.__nde.backend;
    if (typeof t.shownAsks !== "number") return null;
    const a0 = t.shownAsks;
    for (let k = 0; k < 10; k++) {
      const s = { ...spec, columns: [{ col: 3, rule: { type: "values", values: ["Osaka", "Perth", "Lisbon"].slice(0, 1 + (k % 3)), blanks: false } }] };
      t.update(await be.setFilter(t.id, s), false);
      t.heightsInForce();
      await new Promise((r) => setTimeout(r, 40));
    }
    await new Promise((r) => setTimeout(r, 800));
    t.heightsInForce();
    await new Promise((r) => setTimeout(r, 400));
    return t.shownAsks - a0;
  }, spec);
  console.log(JSON.stringify(res));
  await b.close();
})();
