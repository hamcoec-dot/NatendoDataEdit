// UI test of per-row heights against the vite dev server (browser stub backend):
// drag the line under a row number, several rows at once, fit to text, reset,
// Undo and Redo, hit-testing and scrolling with tall rows above, heights
// following rows that are inserted, deleted or moved, hidden rows, sort, the
// freeze line and row-move drag next to the resize band, and heights hidden
// while a filter is on.
// Run: (cd ui && npx vite --port 5215) then
// NODE_PATH=<folder with playwright> node scripts/ui-rowheights.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5215";
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

  const canvas = await (await p.$("canvas")).boundingBox();
  const heights = () => p.evaluate(() => window.__nde.tab.rowHeights.toView());
  const custom = (r) => p.evaluate((r) => window.__nde.tab.rowHeights.custom(r), r);
  // Client coordinates of a cell's bottom-left (the grid's own answer), and of a row number.
  const cellPt = (row) => p.evaluate((r) => window.__nde.grid.cellPoint({ row: r, col: 0 }), row);
  const numberX = 20;
  /** Drag the line under row `row` by dy pixels. */
  const dragEdge = async (row, dy, opts = {}) => {
    const pt = await cellPt(row);
    const y = pt.y - 1;
    await p.mouse.move(numberX, y);
    await p.mouse.down();
    await p.mouse.move(numberX, y + dy / 2, { steps: 3 });
    await p.mouse.move(numberX, y + dy, { steps: 3 });
    if (opts.keep) return;
    await p.mouse.up();
    await p.waitForTimeout(250);
  };
  const clickRowNumber = async (row, shift) => {
    const pt = await cellPt(row);
    if (shift) await p.keyboard.down("Shift");
    await p.mouse.click(numberX, pt.y - 12);
    if (shift) await p.keyboard.up("Shift");
  };
  const cmd = async (q) => {
    await p.keyboard.press("Alt+q");
    await p.keyboard.type(q);
    await p.waitForTimeout(300);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(500);
  };
  const BASE = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 3, col: 0 }).y - window.__nde.grid.cellPoint({ row: 2, col: 0 }).y);
  console.log("INFO plain row height", BASE);

  // ---- resize one row ----
  await p.mouse.click(canvas.x + 300, canvas.y + 100);
  await dragEdge(3, 35);
  const h3 = await custom(3);
  ok(h3 === BASE + 35, `dragging the line under row 4 makes it ${h3} px (plain ${BASE} + 35)`);
  ok((await cellPt(3)).y - (await cellPt(2)).y === h3 && (await cellPt(4)).y - (await cellPt(3)).y === BASE, "the row is that tall and the next one is the plain height");
  if (out) await p.screenshot({ path: path.join(out, "rowheights.png") });
  // The cursor on the line.
  const pt = await cellPt(3);
  await p.mouse.move(numberX, pt.y - 1);
  const cur = await p.$eval(".grid, [aria-label='Data grid']", (e) => getComputedStyle(e).cursor).catch(() => "");
  ok(cur === "row-resize", `the pointer on the line shows a row-resize cursor (${cur})`);
  // Hit-testing and scrolling with a tall row above.
  const below = await cellPt(4);
  await p.mouse.click(canvas.x + 300, below.y - 8);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 4, "clicking in the row below the tall one selects that row");
  await p.mouse.click(canvas.x + 300, pt.y - 20);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 3, "clicking inside the tall row selects it");
  await p.locator("input.goto").fill("R400 C2");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  ok((await p.evaluate(() => window.__nde.tab.cell.row)) === 399, "Go To a row far below the tall one lands on it");
  await p.keyboard.press("Control+Home");
  await p.waitForTimeout(300);

  // ---- undo and redo ----
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await heights()).length === 0, "Ctrl+Z takes the height back");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(400);
  ok((await custom(3)) === BASE + 35, "Ctrl+Y puts it back");

  // ---- several rows at once ----
  await clickRowNumber(6);
  await clickRowNumber(8, true);
  await dragEdge(7, 20);
  const set = await heights();
  const rows678 = [6, 7, 8].every((r) => set.some(([x, px]) => x === r && px === BASE + 20));
  ok(rows678, `the edge of a selected row sizes all selected rows (${JSON.stringify(set)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(6)) === null && (await custom(3)) === BASE + 35, "one Ctrl+Z undoes the whole several-row resize and nothing else");

  // ---- fit to text ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    await window.__nde.backend.setCells(t.id, [[12, 0, "a very long note that will need several lines when this column wraps its text"]]);
    t.update(await window.__nde.backend.docInfo(t.id), true);
    t.columns.setRange(0, 1, 90);
    t.columns.wraps.add(0);
    t.bump();
  });
  await p.waitForTimeout(500);
  const pt12 = await cellPt(12);
  await p.mouse.move(numberX, pt12.y - 1);
  await p.mouse.dblclick(numberX, pt12.y - 1);
  await p.waitForTimeout(500);
  const fitted = await custom(12);
  ok(fitted !== null && fitted > BASE, `double-clicking the line fits the row to its wrapped text (${fitted} px)`);

  // ---- reset ----
  await p.mouse.click(canvas.x + 300, (await cellPt(12)).y - 8);
  await cmd("Reset the height of the selected rows");
  ok((await custom(12)) === null, "Reset puts the selected row back to the plain height");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(12)) !== null, "and Ctrl+Z gets the height back");

  // ---- heights follow inserted and deleted rows ----
  await p.evaluate(() => window.__nde.tab.rowHeights.clear());
  await p.evaluate(() => window.__nde.tab.rowHeights.set([5], 60));
  await p.evaluate(() => window.__nde.tab.bump());
  await p.mouse.click(canvas.x + 300, (await cellPt(2)).y - 8);
  await cmd("Insert row above");
  ok((await custom(5)) === null && (await custom(6)) === 60, `a row inserted above shifts the tall row down with its data (${JSON.stringify(await heights())})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(5)) === 60 && (await custom(6)) === null, "Ctrl+Z puts the height back on the same data row");
  await p.keyboard.press("Control+y");
  await p.waitForTimeout(400);
  ok((await custom(6)) === 60, "Ctrl+Y again");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  // delete the tall row itself
  await p.evaluate(() => window.__nde.tab.update({ ...window.__nde.tab.info }, false));
  await p.mouse.click(canvas.x + 300, (await cellPt(5)).y - 8);
  await cmd("Delete rows");
  ok((await heights()).length === 0, "deleting the tall row removes its height");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(5)) === 60, "Ctrl+Z brings the row and its height back");

  // ---- sort leaves heights in place ----
  await p.mouse.click(canvas.x + 300, (await cellPt(1)).y - 8);
  await cmd("Sort A to Z");
  await p.waitForTimeout(600);
  ok((await custom(5)) === 60, "sorting leaves the heights at their row positions");
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);

  // ---- rows hidden: each shown row keeps its own height, and a row change while hidden keeps them on their rows (gap #4) ----
  await p.mouse.click(canvas.x + 300, (await cellPt(8)).y - 8);
  await p.keyboard.press("Control+9");
  await p.waitForTimeout(500);
  ok(!/Custom row heights hidden while filtered/.test(await status()), "with rows hidden the status bar no longer says the custom heights are hidden");
  const tall = async () => (await cellPt(5)).y - (await cellPt(4)).y;
  for (let i = 0; i < 20 && (await tall()) !== 60; i++) await p.waitForTimeout(100);
  ok((await tall()) === 60, `and the tall row still draws at its height (${await tall()} px; ${JSON.stringify(await p.evaluate(() => [window.__nde.tab.info.rows, window.__nde.tab.heightsInForce()?.toView()]))})`);
  await p.mouse.click(canvas.x + 300, (await cellPt(2)).y - 8);
  await cmd("Insert row above");
  ok(JSON.stringify(await heights()) === "[[6,60]]" && !/custom row heights were cleared/i.test(await status()), `a row inserted above while some rows are hidden moves the height down with its row (${JSON.stringify(await heights())})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(5)) === 60, "Ctrl+Z brings the heights back");
  await p.keyboard.press("Control+Shift+9");
  await p.waitForTimeout(300);

  // ---- every row shown again before the rest ----
  await cmd("Unhide all rows");
  await p.waitForTimeout(300);
  ok(!(await p.evaluate(() => window.__nde.tab.info.filtered)), "(all rows shown again)");

  // ---- a tool that removes rows (Remove blank rows) clears the heights and SAYS so right away ----
  await p.evaluate(async () => {
    const t = window.__nde.tab;
    t.rowHeights.clear();
    t.rowHeights.set([4], 55);
    const cols = t.info.cols;
    await window.__nde.backend.setCells(
      t.id,
      Array.from({ length: cols }, (_, c) => [20, c, ""]),
    );
    t.update(await window.__nde.backend.docInfo(t.id), true);
    t.bump();
  });
  await p.waitForTimeout(400);
  await cmd("Remove blank rows");
  await p.waitForTimeout(300);
  for (const b of ["OK", "Remove", "Apply", "Run"]) {
    const btn = p.getByRole("button", { name: b, exact: true });
    if (await btn.first().isVisible().catch(() => false)) {
      await btn.first().click();
      break;
    }
  }
  await p.waitForTimeout(900);
  ok((await heights()).length === 0, "a tool that removes rows clears the heights it cannot follow");
  ok(/custom row heights were cleared/i.test(await status()), `and the status bar says so without waiting for another action (${(await status()).slice(0, 140)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);

  // ---- a row moved by dragging its number takes its height along ----
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.rowHeights.clear();
    t.rowHeights.set([6], 50);
    t.bump();
  });
  await p.waitForTimeout(300);
  await clickRowNumber(6);
  const from = await cellPt(6);
  const to = await cellPt(10);
  await p.mouse.move(numberX, from.y - 12);
  await p.mouse.down();
  await p.mouse.move(numberX, from.y + 20, { steps: 4 });
  await p.mouse.move(numberX, to.y - 4, { steps: 8 });
  await p.mouse.up();
  await p.waitForTimeout(500);
  const moved = await heights();
  ok(moved.length === 1 && moved[0][0] >= 9 && moved[0][1] === 50, `dragging a tall row to another place carries its height along (${JSON.stringify(moved)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(400);
  ok((await custom(6)) === 50, "and Ctrl+Z puts the height back with the row");

  // ---- frozen rows: the last one's bottom line is the freeze line, the others' lines resize ----
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.rowHeights.clear();
    t.rowHeights.set([1], 60);
    t.frozenRows = 2; // rows 1 and 2 (the titles are row 0)
    t.bump();
  });
  await p.waitForTimeout(300);
  const freezeY = (await cellPt(2)).y; // bottom of the last frozen row
  await p.mouse.move(numberX, freezeY - 1);
  await p.mouse.down();
  await p.mouse.move(numberX, freezeY + 70, { steps: 6 });
  await p.mouse.up();
  await p.waitForTimeout(400);
  const frozenAfter = await p.evaluate(() => window.__nde.tab.frozenRows);
  ok(frozenAfter !== 2 && (await custom(2)) === null && (await custom(1)) === 60, `dragging at the last frozen row's bottom line moves the freeze line (2 -> ${frozenAfter} rows), not a row height`);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.frozenRows = 2;
    t.bump();
  });
  await p.waitForTimeout(300);
  await dragEdge(1, 20);
  ok((await custom(1)) === 80 && (await p.evaluate(() => window.__nde.tab.frozenRows)) === 2, "the bottom line of an earlier, tall frozen row still resizes it");
  await p.evaluate(() => {
    window.__nde.tab.frozenRows = 0;
    window.__nde.tab.bump();
  });

  // ---- a saved view keeps the heights; a row past the file's end is dropped, not an error ----
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.rowHeights.clear();
    t.rowHeights.set([3, 9], 60);
    t.bump();
  });
  await cmd("Save view to a file");
  await p.waitForTimeout(500);
  const saved = await p.evaluate(() => window.__nde.backend.savedView?.row_heights);
  ok(JSON.stringify(saved) === JSON.stringify([[3, 60], [9, 60]]), `a saved view carries the heights (${JSON.stringify(saved)})`);
  await p.evaluate(() => {
    window.__nde.tab.rowHeights.clear();
    window.__nde.tab.bump();
    window.__nde.backend.savedView.row_heights.push([5_000_000, 40], [4, 9999]);
  });
  await cmd("Apply a view from a file");
  await p.waitForTimeout(700);
  const applied = await heights();
  ok(JSON.stringify(applied) === JSON.stringify([[3, 60], [4, 400], [9, 60]]), `applying it puts the heights back (out-of-range ones clamped; ${JSON.stringify(applied)})`);
  ok(/did not fit this file/.test(await status()), `and says a height past the file's end was left out (${(await status()).slice(0, 110)})`);
  await p.keyboard.press("Control+z");
  await p.waitForTimeout(500);
  ok((await heights()).length === 0, "Ctrl+Z takes the applied view's heights back with the view");

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  await b.close();
  process.exit(failed ? 1 : 0);
})();
