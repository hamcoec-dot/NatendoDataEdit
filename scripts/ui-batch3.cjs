// UI test of v0.4.3 (power-user batch 3): the counts tab, the column
// profile, the filter panel and the column insight strip.
// Run: (cd ui && npx vite --port 5202) then
// NODE_PATH=<folder with playwright> node scripts/ui-batch3.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5202";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const wait = (ms) => p.waitForTimeout(ms);
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const cell = (r, c) => p.evaluate(([a, b2]) => window.__nde.tab.cache.get(a, b2), [r, c]);
  const status = () => p.textContent("footer.status").catch(() => "");
  const sel = (r, c, r2 = r, c2 = c) =>
    p.evaluate(([a, b2, c3, d]) => {
      window.__nde.grid.select({ row: a, col: b2 }, { row: c3, col: d });
      window.__nde.grid.focus();
    }, [r, c, r2, c2]).then(() => wait(200));
  const command = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await wait(300);
    await p.keyboard.press("Enter");
    await wait(600);
  };
  const dialog = (title) => p.locator(`[role=dialog]:has(h2:has-text('${title}'))`);
  const tabCount = () => p.evaluate(() => window.__nde.tabs.length);

  await p.goto(url);
  await p.keyboard.press("Control+n");
  await wait(700);
  const rows = ["voter_id\tparty\tprecinct\tzip"];
  const parties = ["D", "R", "D", "I", "D", "R", "D", "", "R", "D"];
  for (let i = 0; i < 120; i++) rows.push(`${1001 + i}\t${parties[i % 10]}\t${i % 3 ? 12 : 3}\t${i === 7 ? "3742" : i === 9 ? "37421-0042" : 37400 + (i % 30)}`);
  await sel(0, 0);
  await clip(rows.join("\r\n") + "\r\n");
  await p.keyboard.press("Control+v");
  await wait(800);
  await p.evaluate(() => {
    window.__nde.tab.header = true;
    window.__nde.tab.bump();
  });

  // ---- Counts tab ----
  await sel(1, 1);
  await p.keyboard.press("Alt+Shift+F");
  await wait(800);
  ok((await tabCount()) === 2, "Alt+Shift+F opens a counts tab");
  ok((await cell(0, 1)) === "Count" && (await cell(0, 2)) === "Percent", "it has Count and Percent");
  ok((await cell(1, 0)) === "D" && (await cell(1, 1)) === "60", `most common first (${await cell(1, 0)} ${await cell(1, 1)})`);
  ok(/different value/.test(await status()), "the status says how many values");
  if (out) await p.screenshot({ path: path.join(out, "counts-tab.png"), clip: { x: 0, y: 0, width: 760, height: 300 } });
  await sel(2, 0);
  await p.keyboard.press("Enter");
  await wait(800);
  const src = await p.evaluate(() => ({ active: window.__nde.tab.id, filtered: window.__nde.tab.info.filtered, filter: JSON.stringify(window.__nde.tab.filter) }));
  ok(src.filtered && /"R"/.test(src.filter), `Enter on R shows R's rows in the source tab (${src.filter})`);
  await command("Clear filter");
  // Two columns: party and precinct together.
  await sel(1, 1, 1, 2);
  await command("Count values in a new tab");
  ok((await tabCount()) === 3 && (await cell(0, 0)) === "party" && (await cell(0, 1)) === "precinct", "two selected columns are counted together");
  await sel(1, 0);
  const pt = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 1, col: 0 }));
  await p.mouse.dblclick(pt.x + 4, pt.y - 6);
  await wait(900);
  ok(await p.evaluate(() => window.__nde.tab.info.filtered), "double-click on a line shows its rows");
  await command("Clear filter");

  // ---- Column profile ----
  await sel(1, 3);
  await command("Column profile");
  const pd = dialog("Profile of");
  ok(await pd.isVisible(), `the column profile opens (${await status()})`);
  const text = await pd.textContent();
  ok(/99999/.test(text) && /9999\b/.test(text), "it lists the shapes, the odd ones too");
  ok(/Shape seen once/.test(text), "a shape seen once is odd");
  if (out) await pd.screenshot({ path: path.join(out, "column-profile.png") });
  await pd.locator("tr.line:has-text('Shape seen once')").first().click();
  await wait(800);
  ok(await p.evaluate(() => window.__nde.tab.info.filtered), "clicking an odd shape shows its rows");
  await command("Clear filter");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
