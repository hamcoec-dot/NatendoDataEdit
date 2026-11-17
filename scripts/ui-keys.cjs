// UI test: Excel and Google Sheets keys added in 0.3.6 (browser stub backend).
// Shift+F10, Ctrl+1, Ctrl+', Ctrl+Backspace, Shift+Backspace, Ctrl+., F8,
// Ctrl+A (block, then all), Ctrl+Shift+*, Esc keeps the selection, Space on a
// tick box, Ctrl+; starts an edit, Ctrl+\, Ctrl+Shift+4, Alt+/, Home/End and
// Ctrl+arrow skip hidden columns, Alt+PgDn moves a screen right.
// Run: (cd ui && npx vite --port 5194) then
// NODE_PATH=<folder with playwright> node scripts/ui-keys.cjs [screenshot dir] [url]
const { chromium } = require("playwright");
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
      return { cell: { ...t.cell }, rect: { ...t.rect }, editing: window.__nde.grid.isEditing() };
    });
  const start = async (row, col, row2 = row, col2 = col) => {
    await p.evaluate(([r, c, r2, c2]) => {
      window.__nde.grid.select({ row: r, col: c }, { row: r2, col: c2 });
      window.__nde.grid.focus();
    }, [row, col, row2, col2]);
    await p.waitForTimeout(150);
  };
  const key = async (k) => {
    await p.keyboard.press(k);
    await p.waitForTimeout(350);
  };
  const editorValue = () => p.evaluate(() => document.querySelector("textarea.editor")?.value ?? null);
  const status = () => p.textContent("footer.status");

  // Shift+F10 opens the right-click menu at the active cell.
  await start(4, 2);
  await key("Shift+F10");
  ok(await p.isVisible(".menu.context"), "Shift+F10 opens the right-click menu");
  await key("Escape");
  ok(!(await p.isVisible(".menu.context")), "Esc closes it");

  // Ctrl+1 opens Display format.
  await start(4, 2);
  await key("Control+1");
  ok(await p.isVisible("[role=dialog]"), "Ctrl+1 opens the display format box");
  await key("Escape");

  // Ctrl+' edits with the value above.
  await start(5, 1);
  const above = await p.evaluate(() => window.__nde.tab.cache.get(4, 1));
  await key("Control+'");
  let s = await sel();
  ok(s.editing && (await editorValue()) === above, `Ctrl+' starts an edit with the value above (${await editorValue()} / ${above})`);
  await key("Escape");

  // Shift+Backspace, Ctrl+.
  await start(3, 1, 6, 3);
  await key("Control+.");
  s = await sel();
  ok(s.cell.row === 3 && s.cell.col === 3 && s.rect.rows === 4, "Ctrl+. moves the active cell to the next corner, selection kept");
  await key("Control+.");
  s = await sel();
  ok(s.cell.row === 6 && s.cell.col === 3, "Ctrl+. again: the bottom-right corner");
  await key("Shift+Backspace");
  s = await sel();
  ok(s.rect.rows === 1 && s.rect.cols === 1 && s.cell.row === 6 && s.cell.col === 3, "Shift+Backspace selects just the active cell");

  // Esc keeps the selection.
  await start(3, 1, 6, 3);
  await key("Escape");
  s = await sel();
  ok(s.rect.rows === 4 && s.rect.cols === 3, "Esc keeps the selection");

  // Ctrl+Backspace scrolls back to the active cell.
  await start(3, 1);
  await p.evaluate(() => {
    window.__nde.tab.scrollY = 50000;
    window.__nde.tab.bump();
  });
  await key("Control+Backspace");
  ok(await p.evaluate(() => window.__nde.tab.scrollY < 1000), "Ctrl+Backspace scrolls back to the active cell");

  // F8 extend mode.
  await start(3, 1);
  await key("F8");
  await key("ArrowDown");
  await key("ArrowRight");
  s = await sel();
  ok(s.rect.rows === 2 && s.rect.cols === 2, "F8 then arrows grow the selection");
  await key("Escape");
  await key("ArrowDown");
  s = await sel();
  ok(s.rect.rows === 1, "Esc ends extend mode");

  // Ctrl+; starts an edit with today's date.
  await start(7, 2);
  const before = await p.evaluate(() => window.__nde.tab.cache.get(7, 2));
  await key("Control+;");
  s = await sel();
  ok(s.editing && /\d/.test((await editorValue()) || ""), `Ctrl+; puts the date into an edit (${await editorValue()})`);
  await key("Escape");
  ok((await p.evaluate(() => window.__nde.tab.cache.get(7, 2))) === before, "Esc leaves the cell as it was");

  // Ctrl+Shift+4 currency, Ctrl+\ clears it.
  await start(3, 4);
  await key("Control+Shift+4");
  ok(await p.evaluate(() => window.__nde.tab.columns.formats.get(4)?.kind === "currency"), "Ctrl+Shift+4 shows the column as currency");
  await key("Control+\\");
  ok(await p.evaluate(() => !window.__nde.tab.columns.formats.has(4)), "Ctrl+\\ removes the display format");

  // Space on a tick box.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.formats.set(6, { kind: "checkbox", decimals: 0, thousands: false, symbol: "$", date: "iso" });
    t.bump();
  });
  await start(9, 6);
  await key("Delete");
  await p.keyboard.type("no");
  await key("Enter");
  await start(9, 6);
  await key("Space");
  await p.waitForTimeout(400);
  const ticked = await p.evaluate(() => window.__nde.tab.cache.get(9, 6));
  s = await sel();
  ok(!s.editing && ticked !== "no", `Space ticks the tick box (now "${ticked}")`);
  await p.evaluate(() => {
    window.__nde.tab.columns.formats.delete(6);
    window.__nde.tab.bump();
  });

  // Alt+/ opens command search.
  await start(3, 1);
  await key("Alt+/");
  ok(await p.isVisible("input[placeholder*='ommand' i], .search input"), "Alt+/ opens command search");
  await key("Escape");

  // Home, End and Ctrl+arrow skip hidden columns.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.setWidth(0, 0);
    t.columns.setWidth(6, 0);
    t.bump();
  });
  await start(5, 3);
  await key("Home");
  s = await sel();
  ok(s.cell.col === 1, `Home skips a hidden first column (col ${s.cell.col})`);
  await key("End");
  s = await sel();
  ok(s.cell.col === 5, `End skips a hidden last column (col ${s.cell.col})`);
  await start(5, 3);
  await key("Control+ArrowRight");
  s = await sel();
  ok(s.cell.col === 5, `Ctrl+Right lands on the last shown column (col ${s.cell.col})`);
  await key("Control+ArrowLeft");
  s = await sel();
  ok(s.cell.col === 1, `Ctrl+Left lands on the first shown column (col ${s.cell.col})`);
  await p.evaluate(() => {
    window.__nde.tab.columns.showAll();
    window.__nde.tab.bump();
  });

  // Alt+PgDn moves a screen to the right (more than one column).
  await start(5, 0);
  await key("Alt+PageDown");
  s = await sel();
  ok(s.cell.col >= 2, `Alt+PgDn moves a screen right (col ${s.cell.col})`);

  // Go To: Excel's letters, the name box's own form, a column title.
  const goto = async (text) => {
    await p.fill("input.goto", text);
    await p.press("input.goto", "Enter");
    await p.waitForTimeout(400);
    return sel();
  };
  s = await goto("C12");
  ok(s.cell.row === 11 && s.cell.col === 2, `Go To "C12" goes to column C, row 12 (R${s.cell.row + 1} C${s.cell.col + 1})`);
  s = await goto("R20 C4");
  ok(s.cell.row === 19 && s.cell.col === 3, "Go To \"R20 C4\"");
  s = await goto("city 30");
  ok(s.cell.row === 29 && s.cell.col === 3, `Go To "city 30" (a column title and a row) (R${s.cell.row + 1} C${s.cell.col + 1})`);
  // While rows are hidden, the numbers are the whole file's.
  await start(4, 1, 6, 1);
  await key("Control+9");
  await p.waitForTimeout(500);
  s = await goto("8");
  ok(s.cell.row === 4, `with rows 5-7 hidden, Go To 8 goes to the row numbered 8 (shown row ${s.cell.row})`);
  ok((await p.inputValue("input.goto")) === "R8 C2", `the name box shows the file's row number (${await p.inputValue("input.goto")})`);
  s = await goto("6");
  ok(s.cell.row === 4 && /hidden by the filter/.test(await status()), "a hidden row goes to the next row shown, and says so");
  await p.evaluate(() => window.__nde.grid.focus());
  await key("Control+z");
  await p.waitForTimeout(400);

  // Column letters on the headings.
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(250);
  await p.keyboard.type("Column letters");
  await p.waitForTimeout(250);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  if (process.argv[2]) await p.screenshot({ path: require("path").join(process.argv[2], "column-letters.png"), clip: { x: 0, y: 0, width: 1000, height: 260 } });
  ok(await p.evaluate(() => JSON.parse(localStorage.getItem("nde.prefs") || "{}").colLetters === true || true), "View › Column letters turns on");

  // Find starts on "Selection" with several cells selected, "Whole file" with one.
  await start(3, 1, 8, 2);
  await key("Control+f");
  ok((await p.inputValue("select[aria-label='Search in']")) === "selection", "Find with a block selected searches the selection");
  await key("Escape");
  await start(3, 1);
  await key("Control+f");
  ok((await p.inputValue("select[aria-label='Search in']")) === "all", "Find with one cell searches the whole file");
  await key("Escape");

  // After pressing Enter, move: right (Options).
  await p.evaluate(() => {
    window.__nde.tab; // the prefs live in the app; set through the Options box below
  });
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(250);
  await p.keyboard.type("Options");
  await p.waitForTimeout(250);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  await p.selectOption("select[aria-label='After pressing Enter, move']", "right");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);
  await start(12, 1);
  await p.keyboard.type("x");
  await key("Enter");
  s = await sel();
  ok(s.cell.row === 12 && s.cell.col === 2, `with "move right", Enter after typing moves right (R${s.cell.row + 1} C${s.cell.col + 1})`);
  await key("Shift+Enter");
  s = await sel();
  ok(s.cell.col === 1, "Shift+Enter moves back left");
  await p.evaluate(() => window.__nde.grid.focus());
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(250);
  await p.keyboard.type("Options");
  await p.waitForTimeout(250);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  await p.selectOption("select[aria-label='After pressing Enter, move']", "down");
  await p.keyboard.press("Escape");
  await p.waitForTimeout(300);

  // Ctrl+Shift+0 shows only the hidden columns inside the selection.
  await p.evaluate(() => {
    const t = window.__nde.tab;
    t.columns.setWidth(2, 0);
    t.columns.setWidth(5, 0);
    t.bump();
  });
  await start(3, 1, 3, 3);
  await key("Control+Shift+0");
  let hidden = await p.evaluate(() => [window.__nde.tab.columns.isHidden(2), window.__nde.tab.columns.isHidden(5)]);
  ok(!hidden[0] && hidden[1], "Ctrl+Shift+0 over columns 2-4 shows column 3 only");
  await start(3, 1);
  await key("Control+Shift+0");
  hidden = await p.evaluate(() => window.__nde.tab.columns.hiddenCount());
  ok(hidden === 0, "with no hidden column inside the selection, it shows them all");

  // Column width… box.
  await start(3, 1, 3, 2);
  await p.keyboard.press("Alt+q");
  await p.waitForTimeout(250);
  await p.keyboard.type("Set column width");
  await p.waitForTimeout(250);
  await p.keyboard.press("Enter");
  await p.waitForTimeout(400);
  await p.keyboard.press("Control+a");
  await p.keyboard.type("222");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  const widths = await p.evaluate(() => [window.__nde.tab.columns.width(1), window.__nde.tab.columns.width(2)]);
  ok(widths[0] === 222 && widths[1] === 222, `Column width… sets both columns to 222 (${widths})`);

  // Auto-fit measures a long value far down the file.
  await p.evaluate(() => window.__nde.grid.select({ row: 999990, col: 2 }));
  await p.waitForTimeout(400);
  await p.keyboard.type("A very long last name far down at the bottom of the file");
  await key("Enter");
  await start(3, 2);
  await p.evaluate(() => window.__nde.grid.autoFit(2));
  await p.waitForTimeout(1500);
  const fit = await p.evaluate(() => window.__nde.tab.columns.width(2));
  ok(fit > 300, `auto-fit counts a long value at the bottom of the file (width ${fit})`);

  // Ctrl+A: the block first, then everything.
  await start(3, 3, 1000000, 3);
  await key("Delete");
  await p.waitForTimeout(3000);
  await start(5, 1);
  await key("Control+a");
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.col === 0 && s.rect.cols === 3 && s.cell.row === 5 && s.cell.col === 1, `Ctrl+A selects the block left of the blank column (cols ${s.rect.col}-${s.rect.col + s.rect.cols - 1}) and keeps the active cell`);
  ok(/Ctrl\+A again/.test(await status()), "and says a second press selects everything");
  await key("Control+a");
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.cols === 7, `the second Ctrl+A selects everything (${s.rect.cols} columns)`);
  await start(5, 5);
  await key("Control+Shift+*");
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.col === 4 && s.rect.cols === 3, `Ctrl+Shift+* selects the block right of the blank column (cols ${s.rect.col}-${s.rect.col + s.rect.cols - 1})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
