// UI test of "Find gaps in a sequence" in the browser stand-in: the dialog opens from the menus and command search,
// lists the missing ranges for a made-up invoice column, copies them, shows the rows next to each gap and opens the
// result as a tab. (The stand-in audits one column simply; the real rules are the engine's tests.)
// Run: (cd ui && npx vite --port 5361) then NODE_PATH=<folder with playwright> node scripts/ui-gaps.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5361";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 1, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const run = async (id) => {
    await p.evaluate((id) => void window.__nde.reg.run(id), id);
    await p.waitForTimeout(500);
  };
  const text = async () => ((await p.locator("body").textContent()) || "").replace(/\s+/g, " ");

  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde.tab && window.__nde.tab.info.indexed && window.__nde.tab.info.rows > 1000, null, { timeout: 30000 });
  await p.waitForTimeout(800);
  // Made-up invoice numbers over the second column: every row in order, three left out.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.header = true;
    const cells = [];
    let n = 1001;
    for (let r = 1; r < t.info.rows; r++) {
      if (n === 1004 || n === 1009 || n === 1010) n += n === 1009 ? 2 : 1;
      cells.push([r, 1, `INV-${String(n++).padStart(5, "0")}`]);
      if (cells.length >= 5000) break;
    }
    cells.push([0, 1, "Invoice"]);
    t.update(await window.__nde.backend.setCells(t.id, cells), true);
    // Keep only the rows made up here.
    t.update(await window.__nde.backend.dataOp(t.id, { op: "show_shape", spec: { col: 1, shape: "AAA-99999", negate: false, include_blanks: false, keep_first: true } }).then((r) => r.info), true);
    t.bump();
    window.__nde.grid.select({ row: 1, col: 1 }, { row: 1, col: 1 });
  });
  await p.waitForTimeout(500);

  await run("data.gaps");
  const dlg = p.getByRole("dialog", { name: "Find gaps in a sequence" });
  await dlg.waitFor({ timeout: 5000 });
  await p.waitForTimeout(600);
  let t = await text();
  ok(/3 missing in 2 ranges/.test(t), "the dialog lists what is missing as soon as it opens");
  ok(/INV-01004 \(1 missing\)/.test(t) && /INV-01009 to INV-01010 \(2 missing\)/.test(t), "as ranges");
  ok(/INV-##### \([\d,]+ rows\)/.test(t), "the shape is listed with its count");
  if (out) await p.screenshot({ path: path.join(out, "gaps.png") });
  await dlg.getByLabel("Start at (optional)").fill("1000");
  ok(await dlg.getByRole("button", { name: "Copy missing list" }).isDisabled(), "a changed option greys the buttons until it is run again");
  await dlg.getByLabel("Start at (optional)").fill("");
  await dlg.getByRole("button", { name: "Copy missing list" }).click();
  await p.waitForTimeout(300);
  const clip = await p.evaluate(() => navigator.clipboard.readText());
  ok(clip.replace(/\r/g, "") === "INV-01004 (1 missing)\nINV-01009 to INV-01010 (2 missing)\n", `copy gives the range lines (${JSON.stringify(clip)})`);
  await dlg.getByRole("button", { name: "Show the rows next to each gap" }).click();
  await p.waitForTimeout(700);
  const shown = await p.evaluate(() => window.__nde.tab.info.rows);
  ok(shown === 5 && /Showing 4 rows next to a gap/.test(await text()), `the rows beside each gap show (${shown} with the title row)`);
  await run("data.clearFilter");
  await run("data.gaps");
  await p.waitForTimeout(600);
  const before = await p.evaluate(() => window.__nde.tab.id);
  await dlg.getByRole("button", { name: "Open as a tab" }).click();
  await p.waitForTimeout(700);
  ok((await p.evaluate(() => window.__nde.tab.id)) !== before, "the result opens as a new tab");
  // The series ticks, each shape, and start and end go into the spec asked for.
  await p.locator('[role="tab"]', { hasText: "sample" }).click();
  await p.waitForTimeout(400);
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 1 }, { row: 1, col: 1 }));
  await run("data.gaps");
  await dlg.waitFor({ timeout: 5000 });
  await p.waitForTimeout(500);
  const spec = () => p.evaluate(() => window.__nde.backend.lastGapSpec);
  await dlg.locator("summary", { hasText: "Series by" }).click();
  const series = dlg.locator(".cols input[type=checkbox]");
  const ticked = () => series.evaluateAll((all) => all.filter((x) => x.checked).length);
  ok((await series.count()) >= 5 && (await ticked()) === 0, "series columns start unticked, the audited column not among them");
  await dlg.getByRole("button", { name: "Select all", exact: true }).click();
  ok((await ticked()) === 2, "Select all ticks at most two");
  await dlg.getByRole("button", { name: "Unselect all", exact: true }).click();
  ok((await ticked()) === 0, "Unselect all clears them");
  for (const i of [0, 1, 2]) await series.nth(i).click();
  ok((await ticked()) === 2, "a third tick keeps two");
  await dlg.getByRole("checkbox", { name: /Audit each shape on its own/ }).check();
  ok(await dlg.getByLabel("Shape", { exact: false }).first().isVisible(), "the controls stay in reach");
  await dlg.getByLabel("Start at (optional)").fill(" INV-01000 ");
  await dlg.getByLabel("End at (optional)").fill("1010");
  await dlg.getByRole("button", { name: "Find gaps" }).click();
  await p.waitForTimeout(500);
  const s = await spec();
  ok(s.series.length === 2 && s.each_shape === true && s.pattern === null && s.start === "INV-01000" && s.end === "1010" && s.col === 1 && s.keep_first === true, `the spec sent (${JSON.stringify(s)})`);
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  await run("data.gaps");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  ok((await p.getByRole("dialog").count()) === 0, "Escape closes the dialog");
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(300);
  await p.keyboard.type("missing numbers in a sequence");
  await p.waitForTimeout(500);
  ok(/Find gaps in a sequence/.test(await text()), "command search finds it by what it does");
  await p.keyboard.press("Escape");
  ok(errors.length === 0, `no page errors (${errors.slice(0, 3).join("; ")})`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
