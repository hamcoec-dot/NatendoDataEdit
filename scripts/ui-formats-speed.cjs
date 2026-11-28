// Speed of the grid drawing display formats, in the browser stand-in, with the
// CPU slowed 20x so the work per frame decides it: 30 visible columns, 2,000
// rows (numbers in even columns, dates with times in odd ones), each case the
// held right arrow, the held down arrow and the wheel, best of 3. Cases: no
// format, every column as number (what main has), a custom code with red
// negatives on the numbers and yyyy-mm-dd hh:mm on the dates, and time.
// Runs on main too (the custom and time cases then fall back to what main
// does with an unknown kind, so compare the number case there).
// Run: (cd ui && npx vite --port 5390) then
// NODE_PATH=<folder with playwright> node scripts/ui-formats-speed.cjs [url]
// (NDE_BROWSER=chromium and NDE_CHROMIUM as in the other suites.) Prints one JSON line.
const { chromium } = require("playwright");
const url = process.argv[2] || "http://localhost:5390";
(async () => {
  const b = await chromium.launch(process.env.NDE_BROWSER === "chromium" ? { executablePath: process.env.NDE_CHROMIUM } : { channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1600, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  const cdp = await ctx.newCDPSession(p);
  const pick = (d) => {
    d.sort((a, c) => a - c);
    return d.length ? { median: +d[Math.floor(d.length / 2)].toFixed(2), p95: +d[Math.floor(d.length * 0.95)].toFixed(2) } : { median: 0, p95: 0 };
  };
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
  const home = async () => {
    await p.evaluate(() => {
      const t = window.__nde.tab;
      t.scrollX = 0;
      t.scrollY = 0;
      window.__nde.grid.select({ row: 1, col: 0 });
      window.__nde.grid.focus();
    });
    await p.waitForTimeout(250);
  };
  const held = (k, n) => async () => {
    for (let i = 0; i < n; i++) {
      await p.keyboard.press(k);
      await p.waitForTimeout(16);
    }
  };
  const best = async (act) => {
    const runs = [];
    for (let i = 0; i < 3; i++) {
      await home();
      runs.push(await measure(act));
    }
    runs.sort((a, c) => a.draw.median - c.draw.median);
    return runs[0];
  };
  // 30 columns x 2,000 rows, pasted into a new file.
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(800);
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 0, col: 0 });
    window.__nde.grid.focus();
  });
  const head = Array.from({ length: 30 }, (_, c) => `c${c + 1}`).join("\t");
  let x = 12345;
  const rnd = () => (x = (x * 1103515245 + 12345) % 2147483648);
  const body = Array.from({ length: 2000 }, () =>
    Array.from({ length: 30 }, (_, c) => (c % 2 === 0 ? `${rnd() % 2 ? "-" : ""}${rnd() % 100000}.${rnd() % 100}` : `2026-0${(rnd() % 9) + 1}-1${rnd() % 9} ${rnd() % 24}:${String(rnd() % 60).padStart(2, "0")}`)).join("\t"),
  ).join("\r\n");
  await p.evaluate((t) => navigator.clipboard.writeText(t), `${head}\r\n${body}\r\n`);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(1500);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    for (let c = 0; c < 30; c++) window.__nde.tab.columns.setWidth(c, 50);
    window.__nde.tab.bump();
  });
  const base = { decimals: 2, thousands: true, symbol: "$", date: "iso", time: "hm12", code: "" };
  const cases = {
    none: () => null,
    number: () => ({ ...base, kind: "number" }),
    custom: (c) => ({ ...base, kind: "custom", code: c % 2 === 0 ? "#,##0.00;[Red]-#,##0.00" : "yyyy-mm-dd hh:mm" }),
    time: () => ({ ...base, kind: "time" }),
  };
  const cv = await (await p.$("canvas")).boundingBox();
  const res = {};
  for (const [name, make] of Object.entries(cases)) {
    await p.evaluate((name) => {
      const t = window.__nde.tab;
      t.columns.formats.clear();
      t.bump();
      window.__case = name;
    }, name);
    await p.evaluate(
      ([fs]) => {
        const t = window.__nde.tab;
        fs.forEach((f, c) => f && t.columns.formats.set(c, f));
        t.bump();
      },
      [Array.from({ length: 30 }, (_, c) => make(c))],
    );
    await p.waitForTimeout(400);
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 20 });
    res[name] = {
      right: await best(held("ArrowRight", 20)),
      down: await best(held("ArrowDown", 60)),
      wheel: await best(async () => {
        await p.mouse.move(cv.x + 400, cv.y + 300);
        for (let i = 0; i < 40; i++) {
          await p.mouse.wheel(0, 240);
          await p.waitForTimeout(16);
        }
      }),
    };
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  }
  console.log(JSON.stringify(res));
  await b.close();
})();
