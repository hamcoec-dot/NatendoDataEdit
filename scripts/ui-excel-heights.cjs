// UI test of gap #7 in the browser stand-in: a workbook's own row heights come
// with its tab (the stand-in's workbook gives heights for rows 0, 1 and 3):
// the title row's is left out, the rows are drawn at those heights, the
// status says how many, Undo right after the import leaves them (it is not a
// step), nothing more is marked changed than a plain import, Reset takes a row
// back to the plain height, and a text import never gets any.
// Run: (cd ui && npx vite --port 5390) then
// NODE_PATH=<folder with playwright> node scripts/ui-excel-heights.cjs [url] [screenshot dir]
const { chromium } = require("playwright");
const path = require("path");
const url = process.argv[2] || "http://localhost:5390";
const shots = process.argv[3];
(async () => {
  // Edge by default; NDE_BROWSER=chromium (NDE_CHROMIUM=path to its chrome.exe when the bundled one is missing).
  const b = await chromium.launch(process.env.NDE_BROWSER === "chromium" ? { executablePath: process.env.NDE_CHROMIUM } : { channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
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
  const status = async () => ((await p.textContent("footer.status")) || "").replace(/\s+/g, " ");
  const importWorkbook = async () => {
    await p.evaluate(() => window.__nde.reg.run("file.import"));
    await p.locator(".shell[role=dialog]").waitFor({ timeout: 5000 });
    await p.waitForTimeout(400);
    await p.getByRole("button", { name: "Import", exact: true }).click();
    await p.waitForTimeout(700);
  };

  // A plain workbook import first: what it marks changed, for comparison.
  await importWorkbook();
  const plain = await p.evaluate(() => ({ modified: window.__nde.tab.info.modified, undo: window.__nde.tab.info.can_undo, heights: window.__nde.tab.rowHeights.toView() }));
  ok(plain.heights.length === 0, "a workbook without heights of its own: none");

  // Now one that has some: 50 px on the title row, 60 px on row 1, 120 px on row 3.
  await p.evaluate(() => (window.__nde.backend.constructor.workbookHeights = [[0, 50], [1, 60], [3, 120]]));
  await importWorkbook();
  const t = await p.evaluate(() => ({ header: window.__nde.tab.header, modified: window.__nde.tab.info.modified, undo: window.__nde.tab.info.can_undo, heights: window.__nde.tab.rowHeights.toView(), steps: window.__nde.tab.lookSteps.length }));
  ok(t.header === true, "the workbook's first row is its titles");
  ok(JSON.stringify(t.heights) === "[[1,60],[3,120]]", `rows 1 and 3 keep their heights; the title row's is left out (${JSON.stringify(t.heights)})`);
  ok(/Row heights from the workbook kept for 2 rows\./.test(await status()), `the status says so (${(await status()).slice(0, 140)})`);
  ok(t.modified === plain.modified && t.undo === plain.undo && t.steps === 0, `no more changed than a plain import, and no undo step (${JSON.stringify(t)})`);
  if (shots) await p.screenshot({ path: path.join(shots, "excel-heights.png") });
  // The Height dialog reads them (as a person sees them).
  await p.evaluate(() => window.__nde.grid.select({ row: 1, col: 0 }));
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("Height of the selected rows");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  const box = p.getByLabel("Height in pixels");
  await box.waitFor({ timeout: 3000 });
  ok((await box.inputValue()) === "60", `the Height dialog shows 60 for row 1 (${await box.inputValue()})`);
  await p.getByRole("button", { name: "Cancel", exact: true }).last().click();
  await p.waitForTimeout(300);
  // Undo right after opening (Ctrl+Z, as a person presses it): the heights stay (opening is not a step).
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok(JSON.stringify(await p.evaluate(() => window.__nde.tab.rowHeights.toView())) === "[[1,60],[3,120]]", "Ctrl+Z right after the import leaves the heights");
  // Reset the height of the selected rows: row 3 back to plain, and that one is a step Ctrl+Z takes back.
  await p.evaluate(() => {
    window.__nde.grid.select({ row: 3, col: 0 });
    window.__nde.grid.focus();
  });
  await p.keyboard.press("Alt+q");
  await p.keyboard.type("Reset the height of the selected rows");
  await p.waitForTimeout(300);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  ok(JSON.stringify(await p.evaluate(() => window.__nde.tab.rowHeights.toView())) === "[[1,60]]", "Reset takes row 3 back to the plain height");
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  {
    const u = JSON.stringify(await p.evaluate(() => window.__nde.tab.rowHeights.toView()));
    ok(u === "[[1,60],[3,120]]", `...and Ctrl+Z brings it back (${u})`);
  }

  // A text import never gets a workbook's heights.
  await p.evaluate(() => navigator.clipboard?.writeText && 0);
  const textHeights = await p.evaluate(async () => {
    const be = window.__nde.backend;
    const s = await be.importText("a,b\n1,2\n3,4\n");
    const d = await be.importFinish(s.id, { format: "delimited", part: 0, raw_dates: false, record: "" });
    return d.row_heights ?? null;
  });
  ok(textHeights === null, "a text import carries no row heights");
  await p.evaluate(() => (window.__nde.backend.constructor.workbookHeights = []));
  ok(errors.length === 0, `no page errors ${errors.join(" | ").slice(0, 200)}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
