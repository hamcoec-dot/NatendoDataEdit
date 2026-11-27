// Speed of the grid while the cursor is moved by a held arrow key, and while
// the wheel scrolls, in the browser preview (Edge, the stub backend). The CPU
// is slowed 20x so the work per frame, not the screen's refresh, decides it.
// Run: (cd ui && npx vite --port 5391) then
// NODE_PATH=<folder with playwright> node scripts/ui-scroll-snap-speed.cjs [url]
// Prints one JSON line: for each case the median and p95 frame and draw times (best of 3).
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5391";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  const cdp = await ctx.newCDPSession(p);
  const pick = (d) => {
    d.sort((a, c) => a - c);
    return d.length ? { median: d[Math.floor(d.length / 2)], p95: d[Math.floor(d.length * 0.95)] } : { median: 0, p95: 0 };
  };
  /** Frame gaps and draw times while `act` runs. */
  const measure = async (act) => {
    await p.evaluate(() => {
      window.__ndeDrawLog = [];
      window.__gaps = [];
      window.__on = true;
      let last = performance.now();
      const tick = (t) => {
        window.__gaps.push(t - last);
        last = t;
        if (window.__on) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await act();
    const r = await p.evaluate(() => {
      window.__on = false;
      const g = window.__gaps.slice(1);
      const d = window.__ndeDrawLog;
      window.__ndeDrawLog = undefined;
      return { g, d };
    });
    return { frame: pick(r.g), draw: pick(r.d) };
  };
  const best = async (act, reset) => {
    const runs = [];
    for (let i = 0; i < 3; i++) {
      await reset();
      runs.push(await measure(act));
    }
    runs.sort((a, c) => a.frame.median - c.frame.median);
    return runs[0];
  };
  const held = (k, n) => async () => {
    for (let i = 0; i < n; i++) {
      await p.keyboard.press(k);
      await p.waitForTimeout(16);
    }
  };
  const home = async () => {
    await p.evaluate(() => {
      const g = window.__nde.grid;
      g.select({ row: window.__nde.tab.firstRow, col: 0 });
      g.focus();
    });
    await p.waitForTimeout(200);
  };
  const res = {};
  // 1M rows x 7 columns (the stub's sample).
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
  await p.waitForTimeout(1500);
  const cv = await (await p.$("canvas")).boundingBox();
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
  res.sample_right = await best(held("ArrowRight", 6), home);
  res.sample_down = await best(held("ArrowDown", 60), home);
  res.sample_wheel = await best(async () => {
    await p.mouse.move(cv.x + 300, cv.y + 200);
    for (let i = 0; i < 60; i++) {
      await p.mouse.wheel(0, 240);
      await p.waitForTimeout(16);
    }
  }, async () => {
    await home();
    await p.evaluate(() => {
      window.__nde.tab.scrollY = 0;
      window.__nde.grid.redraw();
    });
  });
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  // 30 columns, 2,000 rows (pasted in).
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(800);
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 0, col: 0 });
    window.__nde.grid.focus();
  });
  const head = Array.from({ length: 30 }, (_, c) => `c${c + 1}`).join("\t");
  const body = Array.from({ length: 2000 }, (_, r) => Array.from({ length: 30 }, (_, c) => `v${r + 1}.${c + 1}`).join("\t")).join("\r\n");
  await p.evaluate((x) => navigator.clipboard.writeText(x), `${head}\r\n${body}\r\n`);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(1500);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
  res.wide_right = await best(held("ArrowRight", 29), home);
  res.wide_down = await best(held("ArrowDown", 60), home);
  await b.close();
  console.log(JSON.stringify(res));
})();
