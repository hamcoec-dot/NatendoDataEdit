// Speed of a cell drag that leaves the window (gap #6), in the browser stub
// (Edge, CPU slowed 20x, the 1M x 7 sample). A block's outline is dragged and
// the pointer moved 40 times over the tab bar of this window (the same-window
// drag main has), then, on this branch, 40 times outside the window over the
// other "window" (page B). Draw times of the source grid per case (best of 3),
// and how many tabAt asks were made and the most at once.
// Run: (cd ui && npx vite --port 5389) then
// NODE_PATH=<folder with playwright> node scripts/ui-drag-windows-speed.cjs [url]
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5389";
(async () => {
  const br = await chromium.launch({ channel: "msedge" });
  const ctx = await br.newContext({ viewport: { width: 1300, height: 800 } });
  const A = await ctx.newPage();
  const B = await ctx.newPage();
  for (const p of [A, B]) {
    await p.goto(url);
    await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  }
  await B.evaluate(() => window.__nde.reg.run("file.new"));
  await A.keyboard.press("Control+o");
  await A.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
  await A.waitForTimeout(1200);
  await A.getByRole("button", { name: "Yes, column names" }).click({ timeout: 1500 }).catch(() => {});
  await A.evaluate(() => window.__nde.reg.run("file.new"));
  await A.waitForTimeout(400);
  // Back to the sample tab.
  const t0 = await A.locator('[data-tab-index="0"]').boundingBox();
  await A.mouse.click(t0.x + t0.width / 2 - 8, t0.y + t0.height / 2);
  await A.waitForTimeout(600);
  // The layout banner off (it moves the grid down).
  await A.locator(".banner button.close, [aria-label='Close'], button[title='Close']").first().click({ timeout: 800 }).catch(() => {});
  await A.waitForTimeout(300);
  const cdp = await ctx.newCDPSession(A);
  const canvas = await (await A.$("canvas")).boundingBox();
  const pick = (d) => {
    d.sort((a, c) => a - c);
    return d.length ? { median: +d[Math.floor(d.length / 2)].toFixed(2), p95: +d[Math.floor(d.length * 0.95)].toFixed(2), n: d.length } : { median: 0, p95: 0, n: 0 };
  };
  const tab1 = await A.locator('[data-tab-index="1"]').boundingBox();
  // Where the grid's cells are (as the cell-tab suite finds it).
  const HEAD = 25;
  const ROW = 25;
  const W = 130;
  let left = 0;
  for (let x = 30; x < 120; x += 2) {
    await A.mouse.click(canvas.x + x, canvas.y + HEAD + ROW + 12);
    if ((await A.evaluate(() => window.__nde.tab.drawRect.cols)) === 1 && /C1$/.test(await A.inputValue("input.goto"))) {
      left = x;
      break;
    }
  }
  const cx = (c) => canvas.x + left + c * W + W / 2;
  const ry = (r) => canvas.y + HEAD + r * ROW + ROW / 2;
  const width = await A.evaluate(() => window.innerWidth);
  const outsideOk = await A.evaluate(() => typeof window.__nde.backend.tabAt === "function");
  const run = async (outside) => {
    const runs = [];
    for (let k = 0; k < 3; k++) {
      await A.evaluate(() => {
        window.__nde.grid.select({ row: 1, col: 1 }, { row: 2, col: 2 });
        window.__nde.grid.focus();
      });
      await A.waitForTimeout(300);
      const ex = canvas.x + left + W + 1;
      const ey = ry(1);
      await A.mouse.move(ex, ey);
      await A.mouse.down();
      await A.mouse.move(ex + 10, ey + 2, { steps: 2 });
      if (process.env.SHOT && k === 0) await A.screenshot({ path: process.env.SHOT });
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
      await A.evaluate(() => (window.__ndeDrawLog = []));
      for (let i = 0; i < 40; i++) {
        if (outside) {
          await A.evaluate(([x, y]) => window.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y, buttons: 1 })), [width + 40 + (i % 10) * 12, 30 + (i % 3)]);
        } else await A.mouse.move(tab1.x + 10 + (i % 10) * 8, tab1.y + tab1.height / 2);
        await A.waitForTimeout(30);
      }
      await A.waitForTimeout(400);
      const d = await A.evaluate(() => {
        const d = window.__ndeDrawLog;
        window.__ndeDrawLog = undefined;
        return d;
      });
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
      await A.keyboard.press("Escape");
      await A.evaluate(() => window.dispatchEvent(new MouseEvent("mouseup", { clientX: 10, clientY: 10 })));
      await A.mouse.up();
      await A.waitForTimeout(300);
      runs.push(pick(d));
    }
    runs.sort((a, c) => a.median - c.median);
    return runs[0];
  };
  const res = { same_window: await run(false) };
  if (outsideOk) {
    await B.evaluate(() => window.__nde.reg.run("file.new"));
    await A.waitForTimeout(500);
    const c0 = await A.evaluate(() => window.__nde.backend.tabAtCalls);
    res.outside = await run(true);
    res.tabAt = await A.evaluate((c0) => ({ calls: window.__nde.backend.tabAtCalls - c0, most_at_once: window.__nde.backend.tabAtMost }), c0);
  }
  console.log(JSON.stringify(res));
  await br.close();
})();
