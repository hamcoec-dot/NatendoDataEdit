// UI test: the active cell stays where a selection started (as in a
// spreadsheet). After Ctrl+Shift+Down, Shift+arrows, Shift+click, Shift+Space
// or Ctrl+Space, a plain arrow moves from the active cell (browser stub).
// Run: (cd ui && npx vite --port 5193) then
// NODE_PATH=<folder with playwright> node scripts/ui-active.cjs <screenshot dir> [url]
const { chromium } = require("playwright");
const path = require("path");
const out = process.argv[2];
const url = process.argv[3] || "http://localhost:5193";
(async () => {
  const b = await chromium.launch({ channel: "msedge" });
  const p = await b.newPage({ viewport: { width: 1360, height: 860 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(url);
  // The app listens for keys only once it has started.
  await p.waitForFunction(() => window.__nde, null, { timeout: 10000 });
  await p.keyboard.press("Control+o");
  await p.waitForFunction(() => window.__nde?.tab?.info?.indexed, null, { timeout: 30000 });
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
      return { cell: { ...t.cell }, rect: { ...t.drawRect }, last: t.info.rows - 1, cols: t.info.cols };
    });
  const start = async (row, col) => {
    await p.evaluate(([r, c]) => {
      window.__nde.grid.select({ row: r, col: c });
      window.__nde.grid.focus();
    }, [row, col]);
    await p.waitForTimeout(150);
  };
  const key = async (k, n = 1) => {
    for (let i = 0; i < n; i++) await p.keyboard.press(k);
    await p.waitForTimeout(400);
  };

  // Ctrl+Shift+Down, then Down: one below where it started.
  await start(3, 1);
  await key("Control+Shift+ArrowDown");
  let s = await sel();
  ok(s.rect.rows > 100 && s.cell.row === 3 && s.cell.col === 1, `Ctrl+Shift+Down selects ${s.rect.rows} rows; the active cell stays at row 3`);
  await key("ArrowDown");
  s = await sel();
  ok(s.cell.row === 4 && s.cell.col === 1 && s.rect.rows === 1, "then Down goes to row 4, not near the bottom");

  // Ctrl+Shift+Down, then Up: one above the start.
  await start(5, 2);
  await key("Control+Shift+ArrowDown");
  await key("ArrowUp");
  s = await sel();
  ok(s.cell.row === 4 && s.cell.col === 2, "Ctrl+Shift+Down then Up: one above where it started");

  // Shift+arrows grow from the far corner; the active cell stays.
  await start(10, 1);
  await key("Shift+ArrowDown", 3);
  await key("Shift+ArrowRight", 2);
  await p.screenshot({ path: path.join(out, "active-cell-stays.png"), clip: { x: 0, y: 0, width: 1360, height: 560 } });
  s = await sel();
  ok(s.rect.row === 10 && s.rect.rows === 4 && s.rect.cols === 3 && s.cell.row === 10 && s.cell.col === 1, "Shift+arrows grow the block; the active cell stays at its corner");
  await key("Shift+ArrowUp");
  s = await sel();
  ok(s.rect.rows === 3 && s.cell.row === 10, "Shift+Up shrinks it from the far end");
  await key("ArrowRight");
  s = await sel();
  ok(s.cell.row === 10 && s.cell.col === 2 && s.rect.rows === 1, "Right then moves from the active cell");

  // Ctrl+Shift+Down then Ctrl+Shift+Up comes back to the start.
  await start(7, 0);
  await key("Control+Shift+ArrowDown");
  await key("Control+Shift+ArrowUp");
  s = await sel();
  ok(s.cell.row === 7 && s.rect.row <= 7, "Ctrl+Shift+Up after Ctrl+Shift+Down moves the far corner back up");

  // Shift+click: from the active cell to the clicked cell.
  await start(2, 1);
  const box = await p.evaluate(() => {
    const c = document.querySelector("canvas");
    const r = c.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  });
  await p.keyboard.down("Shift");
  await p.mouse.click(box.x + box.w * 0.55, box.y + box.h * 0.6);
  await p.keyboard.up("Shift");
  await p.waitForTimeout(300);
  s = await sel();
  ok(s.rect.rows > 1 && s.cell.row === 2 && s.cell.col === 1, "Shift+click: the active cell stays where it was");
  await p.evaluate(() => window.__nde.grid.focus());
  await key("ArrowDown");
  s = await sel();
  ok(s.cell.row === 3 && s.cell.col === 1, "then Down moves from it");

  // Shift+Space and Ctrl+Space keep the active cell.
  await start(6, 3);
  await key("Shift+Space");
  s = await sel();
  ok(s.rect.cols === s.cols && s.cell.row === 6 && s.cell.col === 3, "Shift+Space selects the row; the active cell stays put");
  await key("ArrowDown");
  s = await sel();
  ok(s.cell.row === 7 && s.cell.col === 3, "then Down goes one below it");
  await start(6, 3);
  await key("Control+Space");
  s = await sel();
  ok(s.rect.rows > 100 && s.cell.row === 6 && s.cell.col === 3, "Ctrl+Space selects the column; the active cell stays put");
  await key("ArrowUp");
  s = await sel();
  ok(s.cell.row === 5 && s.cell.col === 3, "then Up goes one above it, not from the bottom");

  // Moving rows from the keyboard keeps the active cell, moved with them.
  await start(6, 3);
  await key("Shift+Space");
  await key("Alt+Shift+ArrowDown");
  s = await sel();
  ok(s.rect.row === 7 && s.rect.cols === s.cols && s.cell.row === 7 && s.cell.col === 3, "Alt+Shift+Down moves the row; the active cell moves with it (same column)");
  await key("Control+z");

  // Typing after Ctrl+Shift+Down goes into the active cell.
  await start(3, 1);
  await key("Control+Shift+ArrowDown");
  await p.keyboard.type("q");
  await p.waitForTimeout(300);
  const editing = await p.evaluate(() => window.__nde.grid.isEditing());
  s = await sel();
  ok(editing && s.cell.row === 3, "typing edits the active cell (row 3)");
  await key("Escape");

  ok(errors.length === 0, `no page errors ${errors.join("; ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
