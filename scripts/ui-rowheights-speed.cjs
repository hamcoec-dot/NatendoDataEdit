// Speed of the grid with per-row heights, in the browser preview (Edge, canvas,
// the stub backend with a 1,000,001-row file). The frame time is measured with
// the CPU slowed 20x, so the drawing cost (not the screen's refresh) decides it.
// Run: (cd ui && npx vite --port 5215) then
// NODE_PATH=<folder with playwright> node scripts/ui-rowheights-speed.cjs [url] [--no-heights]
// --no-heights: for a build that has no per-row heights (the build from main), to
// compare the plain-rows case.
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5215";
const noHeights = process.argv.includes("--no-heights");
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1000);
  const cdp = await ctx.newCDPSession(p);
  const canvas = await (await p.$("canvas")).boundingBox();
  const frames = () =>
    p.evaluate(async () => {
      const d = [];
      let last = performance.now();
      let n = 0;
      await new Promise((resolve) => {
        const tick = (t) => {
          d.push(t - last);
          last = t;
          if (++n < 120) requestAnimationFrame(tick);
          else resolve();
        };
        requestAnimationFrame(tick);
      });
      d.shift();
      d.sort((a, b) => a - b);
      return { median: d[Math.floor(d.length / 2)], p95: d[Math.floor(d.length * 0.95)] };
    });
  const scrollRun = async () => {
    await p.mouse.move(canvas.x + 300, canvas.y + 200);
    const f = frames();
    for (let i = 0; i < 70; i++) {
      await p.mouse.wheel(0, 240);
      await p.waitForTimeout(16);
    }
    const r = await f;
    for (let i = 0; i < 70; i++) await p.mouse.wheel(0, -240);
    await p.waitForTimeout(300);
    return r;
  };
  const report = (label, r) => console.log(`${label}: median ${r.median.toFixed(1)} ms, p95 ${r.p95.toFixed(1)} ms`);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
  const best = async () => {
    const runs = [];
    for (let i = 0; i < 3; i++) runs.push(await scrollRun());
    runs.sort((a, b) => a.median - b.median);
    return runs[0];
  };
  const zero = await best();
  report("scroll, no custom heights (CPU x20, best of 3)", zero);
  if (!noHeights) {
    for (const n of [1000, 100000]) {
      await p.evaluate((n) => {
        const t = window.__nde.tab;
        t.rowHeights.clear();
        // Spread over the first 400,000 rows, so both the top and a long way down have them.
        const rows = Array.from({ length: n }, (_, i) => Math.floor((i * 400000) / n) + 1);
        t.rowHeights.set(rows, 40);
        t.bump();
      }, n);
      await p.waitForTimeout(500);
      report(`scroll with ${n.toLocaleString()} custom heights`, await best());
    }
    // Jump to the end and back (Ctrl+End), and to a row far down, with 100,000 heights.
    const jump = await p.evaluate(async () => {
      const g = window.__nde.grid;
      const t0 = performance.now();
      g.select({ row: 1_000_000, col: 0 });
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      return performance.now() - t0;
    });
    console.log(`jump to row 1,000,000 with 100,000 heights (CPU x20): ${jump.toFixed(1)} ms (budget 50 ms at full speed = ${(50 * 20).toFixed(0)} ms here)`);
    const fit = await p.evaluate(() => {
      const t = window.__nde.tab;
      const g = window.__nde.grid;
      t.rowHeights.clear();
      g.select({ row: 0, col: 0 });
      const rows = Array.from({ length: 1000 }, (_, i) => i + 1);
      const t0 = performance.now();
      g.fitRowHeights(rows);
      return performance.now() - t0;
    });
    console.log(`fit 1,000 rows (CPU x20): ${fit.toFixed(1)} ms (budget 300 ms at full speed = 6000 ms here)`);

    // 100,000 custom heights with a frozen pane and 1,000 Ctrl+click blocks drawn together.
    await p.evaluate(() => {
      const t = window.__nde.tab;
      t.rowHeights.clear();
      t.rowHeights.set(
        Array.from({ length: 100000 }, (_, i) => Math.floor((i * 400000) / 100000) + 1),
        40,
      );
      t.frozenRows = 3;
      t.extra = Array.from({ length: 1000 }, (_, i) => ({ row: 5 + i * 3, rows: 1, col: i % 4, cols: 1 }));
      t.bump();
    });
    await p.waitForTimeout(500);
    report("scroll with 100,000 heights + a frozen pane + 1,000 blocks", await best());
  }
  console.log(errors.length ? "ERRORS " + errors.join(" | ") : "no page errors");
  await b.close();
})();
