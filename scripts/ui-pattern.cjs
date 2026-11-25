// UI test of the pattern pill in the column insight strip against the vite dev server
// (browser stub backend): the pill is drawn when the column is wide enough, clicking the
// leftover part filters to the rows that break the pattern (and says so), the filter narrows
// one that is already on, the pill's right-click menu has its three commands and Copy pattern
// copies the shape, a click right after a change is refused (the pill is dropped until the
// strip is worked out again), and a narrow column has no pill.
// Run: (cd ui && npx vite --port 5218) then
// NODE_PATH=<folder with playwright> node scripts/ui-pattern.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5218";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const ctx = await b.newContext({ viewport: { width: 1360, height: 860 }, permissions: ["clipboard-read", "clipboard-write"] });
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
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(800);
  // The strip on, the "joined" column wide (the pill needs room beside the figures), 30 odd values among its first rows.
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    if (!window.__nde.prefsOn) await window.__nde.reg.run("view.insights");
    t.columns.setRange(5, 1, 340);
    const cells = Array.from({ length: 30 }, (_, i) => [1 + i * 7, 5, `n/a ${i}`]);
    t.update(await window.__nde.backend.setCells(t.id, cells), true);
    t.bump();
  });
  // The strip is worked out again a second after an edit (and once more with the bigger sample).
  await p.waitForFunction(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.waitForTimeout(600);
  const figures = await p.evaluate(() => {
    const i = window.__nde.tab.insights?.byCol.get(5);
    return { shape: i?.top_shape ?? null, share: i?.top_share ?? 0, revision: window.__nde.tab.insights?.revision, data: window.__nde.tab.info.data_revision };
  });
  ok(figures.shape === "9999-99-99" && figures.share > 0.9 && figures.share < 1, `the strip's figures name the dominant shape (${JSON.stringify(figures)})`);
  ok(figures.revision === figures.data, "and are for the data revision the table has now");
  const canvas = await (await p.$("canvas")).boundingBox();
  const cell = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 1, col: 5 }));
  const left = cell.x - 0; // client x of the column's left edge (the cell's bottom-left corner)
  const stripY = canvas.y + 25 + 11;
  const restX = left + 340 - 25;
  const shapeX = left + 340 - 25 - 105;
  if (out) await p.screenshot({ path: path.join(out, "pattern-pill.png") });

  // ---- the leftover part filters to the rows that break the pattern ----
  await p.mouse.click(restX, stripY);
  await p.waitForTimeout(700);
  const after = await p.evaluate(() => ({ filtered: window.__nde.tab.info.filtered, rows: window.__nde.tab.info.rows }));
  const msg = await status();
  ok(after.filtered && after.rows === 31, `clicking the leftover shows the 30 odd rows (and the title row): filtered ${after.filtered}, ${after.rows} rows`);
  ok(/Showing the 30 rows/.test(msg) && /do not match 9999-99-99/.test(msg) && !/within the current filter/.test(msg), `the status line says what it shows (${msg.slice(0, 160)})`);

  // ---- the same click with a filter already on narrows it ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.clearFilter(t.id), true);
    // A filter that keeps only the first 10 odd rows' neighbourhood: ids below 40.
    const values = Array.from({ length: 40 }, (_, i) => String(i + 1));
    t.update(await window.__nde.backend.setFilter(t.id, { columns: [{ col: 0, rule: { type: "values", values, blanks: false } }], keep_first: true }), true);
  });
  await p.waitForFunction(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.waitForTimeout(800);
  const narrowFigures = await p.evaluate(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape ?? null);
  if (narrowFigures) {
    await p.mouse.click(restX, stripY);
    await p.waitForTimeout(700);
    const n = await p.evaluate(() => window.__nde.tab.info.rows);
    const m2 = await status();
    ok(n < 31 && /within the current filter/.test(m2), `with a filter on, the click narrows it and says so (${n} rows: ${m2.slice(0, 140)})`);
  } else {
    ok(false, "the strip of a filtered table has no pill (the 40 shown rows hold too few odd values to be a pill)");
  }
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.clearFilter(t.id), true);
  });
  await p.waitForTimeout(300);

  // ---- the pill's menu and Copy pattern ----
  await p.waitForFunction(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.waitForTimeout(500);
  await p.mouse.click(shapeX, stripY, { button: "right" });
  await p.waitForTimeout(300);
  const menuText = await p.evaluate(() => Array.from(document.querySelectorAll("[role=menu] [role=menuitem], .menu button")).map((e) => e.textContent.replace(/\s+/g, " ").trim()).join(" | "));
  ok(/Show rows that match/.test(menuText) && /break the column's pattern/.test(menuText) && /Copy the column's pattern/.test(menuText), `the pill's right-click menu has its commands (${menuText.slice(0, 200)})`);
  await p.getByText("Copy the column's pattern").first().click();
  await p.waitForTimeout(300);
  const clip = await p.evaluate(() => navigator.clipboard.readText());
  ok(clip === "9999-99-99", `Copy pattern puts the shape on the clipboard (${clip})`);

  // ---- a change drops the pill until the strip is worked out again ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.setCells(t.id, [[3, 5, "edited"]]), true);
  });
  await p.evaluate(() => window.__nde.reg.run("data.patternOdd"));
  await p.waitForTimeout(300);
  ok(!(await p.evaluate(() => window.__nde.tab.info.filtered)), "right after a change a pill command does not run on the old figures");
  await p.waitForFunction(() => window.__nde.tab.insights?.revision === window.__nde.tab.info.data_revision && window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.evaluate(() => window.__nde.reg.run("data.patternOdd"));
  await p.waitForTimeout(700);
  const again = await p.evaluate(() => ({ filtered: window.__nde.tab.info.filtered, rows: window.__nde.tab.info.rows }));
  ok(again.filtered && again.rows === 32, `once the strip is worked out again the command runs (${JSON.stringify(again)})`);
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.update(await window.__nde.backend.clearFilter(t.id), true);
  });

  // ---- a narrow column has no pill: the same click opens the profile instead ----
  await p.evaluate(() => {
    window.__nde.tab.columns.setRange(5, 1, 110);
    window.__nde.tab.bump();
  });
  await p.waitForFunction(() => window.__nde.tab.insights?.byCol.get(5)?.top_shape, null, { timeout: 15000 }).catch(() => {});
  await p.waitForTimeout(600);
  await p.mouse.click(left + 110 - 15, stripY);
  await p.waitForTimeout(500);
  ok(!(await p.evaluate(() => window.__nde.tab.info.filtered)), "in a narrow column the strip click does not filter");

  ok(errors.length === 0, errors.length ? "page errors: " + errors.join(" | ") : "no page errors");
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
