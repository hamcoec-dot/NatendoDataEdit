// UI test: Excel-style typing and moving (browser stub backend).
// - Enter mode: after typing, arrows finish the edit and move; F2 switches.
// - Enter and Tab move inside a selected block; Tab, Tab, Enter returns to
//   the starting column.
// - Clicking a column or row heading makes the top or first cell active.
// Run: (cd ui && npx vite --port 5194) then
// NODE_PATH=<folder with playwright> node scripts/ui-excel.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5194";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => !/Loading|Indexing|Reading/i.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
  await p.waitForTimeout(1500);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const sel = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      return { cell: { ...t.cell }, rect: { ...t.rect }, first: t.firstRow, editing: window.__nde.grid.isEditing() };
    });
  const start = async (row, col, row2 = row, col2 = col) => {
    await p.evaluate(([r, c, r2, c2]) => {
      window.__nde.grid.select({ row: r, col: c }, { row: r2, col: c2 });
      window.__nde.grid.focus();
    }, [row, col, row2, col2]);
    await p.waitForTimeout(150);
  };
  const key = async (k, n = 1) => {
    for (let i = 0; i < n; i++) await p.keyboard.press(k);
    await p.waitForTimeout(350);
  };
  const bar = () => p.inputValue("textarea.cellvalue");
  const at = async (s, row, col) => s.cell.row === row && s.cell.col === col;

  // ---- 1. Enter mode ----
  await start(2, 1);
  await p.keyboard.type("typed one");
  await key("ArrowRight");
  let s = await sel();
  ok(!s.editing && (await at(s, 2, 2)), "typing then Right: the edit is kept and the next cell is active");
  await key("ArrowLeft");
  ok((await bar()) === "typed one", `the typed value is in the cell (${await bar()})`);
  await p.keyboard.type("down");
  await key("ArrowDown");
  s = await sel();
  ok(!s.editing && (await at(s, 3, 1)), "typing then Down moves down");
  // Edit mode (F2): arrows move inside the text.
  await start(2, 1);
  await key("F2");
  await key("ArrowLeft");
  s = await sel();
  ok(s.editing && (await at(s, 2, 1)), "F2 then Left stays in the cell (Edit mode)");
  await key("Escape");
  // F2 while typing switches to Edit mode.
  await start(4, 1);
  await p.keyboard.type("ab");
  await key("F2");
  await key("ArrowLeft");
  s = await sel();
  ok(s.editing && (await at(s, 4, 1)), "typing, F2, Left: still editing (F2 switched modes)");
  await p.keyboard.type("X");
  await key("Enter");
  await start(4, 1);
  ok((await bar()) === "aXb", `the caret moved inside the text (${await bar()})`);

  // Ctrl+arrow while typing keeps the text and jumps to the edge of the data.
  await start(5, 1);
  await p.keyboard.type("jump");
  await key("Control+ArrowDown");
  s = await sel();
  ok(!s.editing && s.cell.row > 1000 && s.cell.col === 1, `typing then Ctrl+Down: kept, and jumped to row ${s.cell.row}`);
  ok((await (async () => { await start(5, 1); return bar(); })()) === "jump", "the typed value was kept");

  // ---- 2. Enter and Tab inside a block ----
  await start(10, 1, 12, 2);
  await key("Enter");
  s = await sel();
  ok((await at(s, 11, 1)) && s.rect.rows === 3 && s.rect.cols === 2, "Enter in a block: down one, the block stays");
  await key("Enter", 2);
  s = await sel();
  ok((await at(s, 10, 2)) && s.rect.rows === 3, "Enter at the bottom wraps to the top of the next column");
  await key("Shift+Enter");
  s = await sel();
  ok(await at(s, 12, 1), "Shift+Enter goes back up (wrapping to the previous column)");
  await start(10, 1, 12, 2);
  await key("Tab");
  s = await sel();
  ok(await at(s, 10, 2), "Tab in a block: right one");
  await key("Tab");
  s = await sel();
  ok((await at(s, 11, 1)) && s.rect.rows === 3, "Tab at the end of a row wraps to the next row");
  // Typing into a block: Enter moves on inside it.
  await p.keyboard.type("in block");
  await key("Enter");
  s = await sel();
  ok((await at(s, 12, 1)) && s.rect.rows === 3 && s.rect.cols === 2, "typing then Enter: next cell of the block, block kept");
  await p.screenshot({ path: path.join(out, "enter-in-block.png"), clip: { x: 0, y: 0, width: 1360, height: 560 } });

  // A block whose columns are all hidden: Enter stays put.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.setWidth(5, 0);
    t.columns.setWidth(6, 0);
    t.bump();
  });
  await start(10, 5, 11, 6);
  await key("Enter");
  s = await sel();
  ok(await at(s, 10, 5), "Enter in a block with every column hidden stays put");
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.showAll();
    t.bump();
  });

  // ---- 3. Tab, Tab, Enter ----
  await start(20, 1);
  await key("Tab", 2);
  s = await sel();
  ok(await at(s, 20, 3), "Tab, Tab: two to the right");
  await key("Enter");
  s = await sel();
  ok(await at(s, 21, 1), "then Enter: next row, back in the column where the Tabs started");
  await start(20, 1);
  await p.keyboard.type("a");
  await key("Tab");
  await p.keyboard.type("b");
  await key("Tab");
  await p.keyboard.type("c");
  await key("Enter");
  s = await sel();
  ok(await at(s, 21, 1), "typing a row with Tab, then Enter: back to the first column");
  await start(30, 1);
  await key("Tab");
  await key("ArrowDown");
  await key("Enter");
  s = await sel();
  ok(await at(s, 32, 2), "another move ends the run of Tabs (Enter goes straight down)");

  // ---- 4. Heading clicks ----
  const canvas = await (await p.$("canvas")).boundingBox();
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.scrollY = 0;
    t.scrollX = 0;
    t.bump();
  });
  await p.mouse.click(canvas.x + canvas.width * 0.4, canvas.y + 12);
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.rows > 100 && s.cell.row === s.first, `clicking a column heading: the top cell is active (row ${s.cell.row})`);
  await p.mouse.click(canvas.x + 12, canvas.y + 25 * 4 + 12);
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.cols > 1 && s.cell.col === 0, `clicking a row number: the first cell is active (col ${s.cell.col})`);
  const was = s.cell;
  await p.keyboard.press("Control+a");
  await p.waitForTimeout(300);
  await p.keyboard.press("Control+a");
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.row <= s.first && s.rect.cols > 1 && s.cell.row === was.row && s.cell.col === was.col, "Ctrl+A selects everything and the active cell stays where it was (as in Excel)");

  // ---- 5. Ctrl+Minus / Ctrl+Plus on part of a row ask how ----
  const valueAt = async (r, c) => {
    await start(r, c);
    await p.waitForTimeout(150);
    return bar();
  };
  const dlg = "[role=dialog]";
  const below = await valueAt(4, 1);
  const rowsBefore = await p.evaluate(() => window.__nde.tab.info.rows);
  await start(2, 1, 3, 2);
  await key("Control+-");
  ok(await p.isVisible(`${dlg}:has-text('Shift cells up')`), "Ctrl+Minus on part of two rows asks how to delete");
  ok(await p.isChecked(`${dlg} input[value=rows]`), "Entire rows is ticked to start");
  ok((await p.evaluate(() => document.activeElement?.getAttribute("value"))) === "rows", "and has the keyboard focus");
  ok(!(await p.isVisible(`${dlg} .warn`)), "no warning while Entire rows is ticked");
  await p.check(`${dlg} input[value=up]`);
  ok(/other rows/.test((await p.textContent(`${dlg} .warn`).catch(() => "")) || ""), "picking Shift cells up warns that values move into other rows");
  await p.screenshot({ path: path.join(out, "delete-cells.png"), clip: { x: 0, y: 0, width: 1360, height: 600 } });
  await p.click(`${dlg} button.primary`);
  await p.waitForTimeout(400);
  ok(await p.isVisible("[role=alertdialog], [role=dialog]:has-text('Shift cells in')"), "shifting a million rows asks once more");
  await p.click("button:has-text('Shift cells up')");
  await p.waitForTimeout(1500);
  ok((await valueAt(2, 1)) === below, `Shift cells up: the cell two rows down moved up (${await bar()})`);
  ok((await p.evaluate(() => window.__nde.tab.info.rows)) === rowsBefore, "no rows were deleted");
  await key("Control+z");
  await p.waitForTimeout(800);
  ok((await valueAt(4, 1)) === below, "Ctrl+Z puts the cells back");
  await start(2, 1, 2, 2);
  await key("Control+Shift+=");
  ok(await p.isVisible(`${dlg}:has-text('Shift cells right')`), "Ctrl+Plus on part of a row asks how to insert");
  await key("Escape");
  ok(!(await p.isVisible(dlg)), "Esc cancels it");
  await start(2, 0);
  await key("Shift+Space");
  await key("Control+-");
  ok(!(await p.isVisible(dlg)) && (await p.evaluate(() => window.__nde.tab.info.rows)) === rowsBefore - 1, "whole rows selected: Ctrl+Minus deletes them without asking");
  await key("Control+z");
  await p.waitForTimeout(500);

  // ---- 6. Clearing and filling any number of cells ----
  const far = await valueAt(600000, 2);
  await p.evaluate(() => {
    const t = window.__nde.tab;
    window.__nde.grid.select({ row: t.firstRow, col: 2 }, { row: t.info.rows - 1, col: 2 });
    window.__nde.grid.focus();
  });
  await key("Delete");
  await p.waitForFunction(() => /Cleared/.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 60000 }).catch(() => {});
  ok(/Cleared [\d,]{7,} cells/.test(await p.textContent("footer.status")), `Delete on a whole column of a million rows clears it (${((await p.textContent("footer.status")) || "").slice(0, 80)})`);
  ok((await valueAt(600000, 2)) === "" && far !== "", "a cell far down is empty now");
  await key("Control+z");
  await p.waitForTimeout(3000);
  ok((await valueAt(600000, 2)) === far, "Ctrl+Z brings the whole column back");
  await start(1, 4, 200000, 4);
  await p.keyboard.type("z");
  await key("Control+Enter");
  await p.waitForFunction(() => /Filled/.test(document.querySelector("footer.status")?.textContent || ""), null, { timeout: 60000 }).catch(() => {});
  ok((await valueAt(150000, 4)) === "z", "Ctrl+Enter fills 200,000 cells");
  await key("Control+z");
  await p.waitForTimeout(2000);

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
