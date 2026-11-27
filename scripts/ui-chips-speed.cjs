// Speed of the grid with the filter chips bar shown (C8), in the browser preview (Edge, canvas, the stub backend's
// 1,000,001-row file, CPU slowed 20x): scrolling with a 2-rule filter on, the bar hidden (View › Filter chips off)
// and shown, best of 3 runs each, side by side. The bar must not slow the drawing.
// Run: (cd ui && npx vite --port 5321) then NODE_PATH=<folder with playwright> node scripts/ui-chips-speed.cjs [url]
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5321";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 } });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(1000);
  await p.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    const spec = {
      columns: [
        { col: 1, rule: { type: "values", values: ["Ada", "Bo", "Cy", "Dee", "Eli"], blanks: false } },
        { col: 3, rule: { type: "values", values: ["Dayton", "Lisbon", "Osaka", "Perth"], blanks: false } },
      ],
      keep_first: true,
    };
    t.update(await window.__nde.backend.setFilter(t.id, spec), true);
    t.filter = spec;
  });
  await p.waitForSelector("[data-testid=filter-chips]", { timeout: 10000 });
  await p.waitForTimeout(1500);
  const cdp = await ctx.newCDPSession(p);
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
    const canvas = await (await p.$("canvas")).boundingBox();
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
  const best = async () => {
    const runs = [];
    for (let i = 0; i < 3; i++) runs.push(await scrollRun());
    runs.sort((a, c) => a.median - c.median);
    return runs[0];
  };
  const setBar = async (on) => {
    const now = await p.evaluate(() => window.__nde.reg.get("view.filterChips").checked());
    if (now !== on) await p.evaluate(() => window.__nde.reg.run("view.filterChips"));
    await p.waitForTimeout(800);
  };
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
  const rows = [];
  for (let round = 0; round < 2; round++) {
    await setBar(false);
    rows.push(["hidden", await best()]);
    await setBar(true);
    rows.push(["shown", await best()]);
  }
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  for (const [label, r] of rows) console.log(`filter on, chips bar ${label}: frame median ${r.median.toFixed(1)} ms, p95 ${r.p95.toFixed(1)} ms (CPU x20, best of 3)`);
  const med = (l) => rows.filter((x) => x[0] === l).map((x) => x[1].median);
  const h = Math.min(...med("hidden"));
  const s = Math.min(...med("shown"));
  const pass = s <= h * 1.05 + 0.5;
  console.log(`${pass ? "PASS" : "FAIL"} the bar does not slow scrolling (best median shown ${s.toFixed(1)} ms vs hidden ${h.toFixed(1)} ms)`);
  await b.close();
  process.exit(pass ? 0 : 1);
})();
