// UI test: the grid scrolls to whole columns and rows when it follows the
// cursor (keyboard, Go to, Find), as spreadsheets do; a mouse press does not
// snap (the clicked cell stays under the pointer); the wheel still scrolls by
// pixels. Browser stub, made-up data.
// Run: (cd ui && npx vite --port 5391) then
// NODE_PATH=<folder with playwright> node scripts/ui-scroll-snap.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2] && !process.argv[2].startsWith("http") ? process.argv[2] : null;
const url = process.argv.find((a) => a.startsWith("http")) || "http://localhost:5391";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  // About the window of the report: 551 px wide (964 device px at 1.75).
  const ctx = await b.newContext({ viewport: { width: 551, height: 700 }, permissions: ["clipboard-read", "clipboard-write"] });
  const p = await ctx.newPage();
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const shot = (n) => (out ? p.screenshot({ path: path.join(out, n) }) : null);
  await p.goto(url);
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  const settle = () => p.waitForTimeout(120);
  const sel = (r, c) =>
    p.evaluate(([a, b2]) => {
      window.__nde.grid.select({ row: a, col: b2 });
      window.__nde.grid.focus();
    }, [r, c]).then(settle);
  const clip = (t) => p.evaluate((x) => navigator.clipboard.writeText(x), t);
  const key = async (k, n = 1) => {
    for (let i = 0; i < n; i++) await p.keyboard.press(k);
    await settle();
  };
  /** The scroll, the boundaries it may stop on, and where the active cell is. */
  const state = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      const g = window.__nde.grid;
      const fc = Math.min(t.frozenCols, t.info.cols);
      const base = t.columns.x(fc);
      const xs = [];
      for (let c = fc; c <= t.info.cols; c++) xs.push(t.columns.x(c) - base);
      const cv = document.querySelector("canvas").getBoundingClientRect();
      const at = g.cellPoint(t.cell);
      return {
        x: t.scrollX,
        y: t.scrollY,
        xs,
        cell: { ...t.cell },
        anchor: { ...t.sel.anchor },
        focus: { ...t.sel.focus },
        cellLeft: at.x - 4 - cv.left,
        cellRight: at.x - 4 - cv.left + t.columns.width(t.cell.col),
        cellBottom: at.y - cv.top,
        canvasW: cv.width,
        canvasH: cv.height,
      };
    });

  // ---- 1. The reported case: id,first,last,ward over 20 rows ---------------
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(800);
  await sel(0, 0);
  const rows20 = Array.from({ length: 20 }, (_, i) => `${i + 1}\t\t\tW${(i % 4) + 1}`).join("\r\n");
  await clip(`id\tfirst\tlast\tward\r\n${rows20}\r\n`);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(600);
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 4, "a 4-column table is pasted in");
  await sel(1, 0);
  await key("ArrowRight", 3);
  let s = await state();
  ok(s.cell.col === 3, "the cursor is in the ward column");
  ok(s.xs.includes(s.x) && s.x > 0, `ArrowRight ×3 scrolls to a column boundary (${s.x} px; boundaries ${s.xs.slice(0, 5).join(", ")})`);
  ok(s.x === 130, `...to the start of column 2, so column 1 is wholly out of view, not cut (${s.x})`);
  ok(s.cellRight <= s.canvasW + 0.5, `the ward column is whole on screen (right edge ${s.cellRight.toFixed(0)} of ${s.canvasW})`);
  await shot("snap-4-columns.png");
  await key("Home");
  s = await state();
  ok(s.x === 0, `Home scrolls back to the start (${s.x})`);

  // ---- 2. Mouse: no snap under the pointer (S1) ----------------------------
  // At scroll 0 the ward column is cut by the right edge: press in its shown part.
  const cv = await (await p.$("canvas")).boundingBox();
  const wardLeft = await p.evaluate(() => {
    const g = window.__nde.grid;
    const cvr = document.querySelector("canvas").getBoundingClientRect();
    return g.cellPoint({ row: 3, col: 3 }).x - 4 - cvr.left;
  });
  const px = cv.x + wardLeft + 20;
  const py = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 3, col: 3 }).y - 10);
  await p.mouse.move(px, py);
  await p.mouse.down();
  await settle();
  s = await state();
  const pressX = s.x;
  ok(s.anchor.row === 3 && s.anchor.col === 3, `a press on a part-shown cell selects it (R${s.anchor.row + 1} C${s.anchor.col + 1})`);
  ok(pressX < 130 && pressX > 0, `the press moves the view only by the hidden part (${pressX} px, not a whole column)`);
  ok(s.cellLeft <= px - cv.x && px - cv.x < s.cellRight, "the pressed cell is still under the pointer");
  // Drag two rows down and back left one column: the block starts at the pressed cell.
  await p.mouse.move(px - 130, py + 2 * 24, { steps: 4 });
  await p.mouse.up();
  await settle();
  s = await state();
  ok(s.anchor.row === 3 && s.anchor.col === 3, `the drag-select is anchored at the pressed cell (R${s.anchor.row + 1} C${s.anchor.col + 1} to R${s.focus.row + 1} C${s.focus.col + 1})`);
  ok(s.focus.col === 2 && s.focus.row > 3, `...and reaches the cell the pointer went to (R${s.focus.row + 1} C${s.focus.col + 1})`);
  // A double-click on a part-shown cell opens the editor on that cell.
  await p.evaluate(() => {
    window.__nde.tab.scrollX = 0;
    window.__nde.grid.redraw();
  });
  await sel(1, 0);
  const py2 = await p.evaluate(() => window.__nde.grid.cellPoint({ row: 5, col: 3 }).y - 10);
  await p.mouse.dblclick(px, py2);
  await p.waitForTimeout(300);
  s = await state();
  const editing = await p.evaluate(() => window.__nde.grid.isEditing());
  ok(editing && s.cell.row === 5 && s.cell.col === 3, `a double-click on a part-shown cell edits that cell (R${s.cell.row + 1} C${s.cell.col + 1}, editing ${editing})`);
  await p.keyboard.press("Escape");
  await settle();

  // ---- 3. 30 columns of mixed widths: a walk right, then Home -------------
  await p.keyboard.press("Control+n");
  await p.waitForTimeout(800);
  await sel(0, 0);
  const head = Array.from({ length: 30 }, (_, c) => `c${c + 1}`).join("\t");
  const body = Array.from({ length: 60 }, (_, r) => Array.from({ length: 30 }, (_, c) => `v${r + 1}.${c + 1}`).join("\t")).join("\r\n");
  await clip(`${head}\r\n${body}\r\n`);
  await p.keyboard.press("Control+v");
  await p.waitForTimeout(800);
  ok((await p.evaluate(() => window.__nde.tab.info.cols)) === 30, "a 30-column table is pasted in");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    for (const [c, w] of [[3, 60], [7, 300], [12, 45], [20, 220], [25, 0]]) t.columns.setWidth(c, w);
    t.bump();
  });
  await sel(1, 0);
  let walkOk = true;
  let wholeOk = true;
  for (let i = 0; i < 29; i++) {
    await p.keyboard.press("ArrowRight");
    await p.waitForTimeout(40);
    s = await state();
    if (!s.xs.includes(s.x)) {
      walkOk = false;
      console.log(`  step ${i + 1}: C${s.cell.col + 1} scroll ${s.x} is not a boundary`);
    }
    if (s.cellLeft < 0 || s.cellRight > s.canvasW + 0.5) {
      wholeOk = false;
      console.log(`  step ${i + 1}: C${s.cell.col + 1} is cut (${s.cellLeft.toFixed(0)}..${s.cellRight.toFixed(0)})`);
    }
  }
  ok(walkOk, "every step of the walk right stops on a column boundary (a whole column at the left)");
  ok(wholeOk, "...with the cursor's column whole on screen");
  ok(s.cell.col === 29, `the walk reaches the last column (C${s.cell.col + 1})`);
  await shot("snap-30-columns-end.png");
  await key("Home");
  ok((await state()).x === 0, "Home goes back to scroll 0");

  // ---- 4. The wheel and the scrollbar still scroll by pixels --------------
  await p.mouse.move(cv.x + 200, cv.y + 200);
  await p.mouse.wheel(33, 0);
  await settle();
  s = await state();
  ok(s.x === 33, `a 33 px sideways wheel stays at 33 px, not snapped (${s.x})`);
  await p.mouse.wheel(100000, 0);
  await settle();
  s = await state();
  const content = s.xs[s.xs.length - 1];
  const blank = s.x - (content - (s.canvasW - (await p.evaluate(() => {
    const g = window.__nde.grid;
    const cvr = document.querySelector("canvas").getBoundingClientRect();
    const t = window.__nde.tab;
    t.scrollX = 0;
    const left = g.cellPoint({ row: 1, col: 0 }).x - 4 - cvr.left;
    return left;
  }))));
  ok(blank >= 0 && blank <= Math.max(220, 40) + 0.5, `the wheel reaches the end, with at most one column of blank space after it (${blank.toFixed(0)} px)`);

  // ---- 5. Rows: ArrowDown, Page Down, Ctrl+End, frozen rows ---------------
  await sel(1, 0);
  const rowH = await p.evaluate(() => {
    const g = window.__nde.grid;
    return g.cellPoint({ row: 2, col: 0 }).y - g.cellPoint({ row: 1, col: 0 }).y;
  });
  let rowsOk = true;
  for (let i = 0; i < 40; i++) {
    await p.keyboard.press("ArrowDown");
    await p.waitForTimeout(30);
    s = await state();
    if (s.y % rowH !== 0) {
      rowsOk = false;
      console.log(`  down ${i + 1}: R${s.cell.row + 1} scroll ${s.y} is not on a row (rows of ${rowH} px)`);
    }
  }
  ok(rowsOk, `every ArrowDown stops with the top row whole (rows of ${rowH} px)`);
  await key("Control+Home");
  await key("PageDown");
  s = await state();
  ok(s.y % rowH === 0, `Page Down: the top row is whole (${s.y} px)`);
  await key("Control+End");
  s = await state();
  ok(s.y % rowH === 0 && s.y > 0, `Ctrl+End: the top row is whole (${s.y} px)`);
  ok(s.cellBottom <= s.canvasH + 0.5, "...and the last row is whole on screen");
  await key("Control+Home");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.frozenRows = 3;
    t.bump();
  });
  await key("PageDown");
  await key("PageDown");
  s = await state();
  ok(s.y % rowH === 0 && s.y > 0, `three frozen rows, Page Down ×2: the first row under them is whole (${s.y} px)`);
  await shot("snap-rows-frozen.png");
  // Custom heights: rows 10 and 20 at 70 px; a walk down stops on row tops.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.frozenRows = 0;
    t.rowHeights.set([10, 20], 70);
    t.bump();
  });
  await sel(1, 0);
  let customOk = true;
  for (let i = 0; i < 40; i++) {
    await p.keyboard.press("ArrowDown");
    await p.waitForTimeout(30);
    const y = (await state()).y;
    const r = await p.evaluate(([y, plain]) => {
      // Row tops below the title row, with the plain height measured before.
      const t = window.__nde.tab;
      let top = 0;
      for (let row = t.firstRow; row < t.firstRow + 70; row++) {
        if (Math.abs(top - y) < 0.01) return true;
        top += t.rowHeights.heightOf(row, plain);
        if (top > y + 0.01) return false;
      }
      return false;
    }, [y, rowH]);
    if (!r) {
      customOk = false;
      console.log(`  custom heights, down ${i + 1}: scroll ${y} is not a row top`);
    }
  }
  ok(customOk, "with custom row heights every ArrowDown stops on a row top");

  ok(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASSED");
  process.exit(failed ? 1 : 0);
})();
