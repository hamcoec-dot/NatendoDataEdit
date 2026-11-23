// UI test: menus and mouse, Excel and Google Sheets style (browser stub backend).
// Alt+Down pick list of a column's values, double-click on the selection's
// border, sort arrows on headings, right-click menus that say how many, the
// fill handle dragged back to clear, Enter in the cell bar moves down, the
// heading arrow (filter buttons off), link titles, list arrows on every cell.
// Run: (cd ui && npx vite --port 5194) then
// NODE_PATH=<folder with playwright> node scripts/ui-mouse.cjs [screenshot dir] [url]
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
  // The title-row question floats over the bottom rows: close it, keeping the guess.
  await p.locator(".header-ask button.close").click({ timeout: 2000 }).catch(() => {});
  await p.waitForTimeout(400);
  let failed = 0;
  const ok = (cond, msg) => {
    if (!cond) failed++;
    console.log(`${cond ? "PASS" : "FAIL"} ${msg}`);
  };
  const sel = () =>
    p.evaluate(() => {
      const t = window.__nde.tab;
      return { cell: { ...t.cell }, rect: { ...t.drawRect }, editing: window.__nde.grid.isEditing() };
    });
  const start = async (row, col, row2 = row, col2 = col) => {
    await p.evaluate(([r, c, r2, c2]) => {
      window.__nde.grid.select({ row: r, col: c }, { row: r2, col: c2 });
      window.__nde.grid.focus();
    }, [row, col, row2, col2]);
    await p.waitForTimeout(200);
  };
  const key = async (k) => {
    await p.keyboard.press(k);
    await p.waitForTimeout(350);
  };
  const command = async (label) => {
    await p.keyboard.press("Alt+q");
    await p.waitForTimeout(250);
    await p.keyboard.type(label);
    await p.waitForTimeout(250);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(400);
  };
  /** A cell's box on screen (client pixels). */
  const box = (row, col) =>
    p.evaluate(([r, c]) => {
      const g = window.__nde.grid;
      const t = window.__nde.tab;
      const a = g.cellPoint({ row: r, col: c });
      const below = g.cellPoint({ row: r + 1, col: c });
      const h = below.y - a.y || 25;
      return { x: a.x - 4, y: a.y - h, w: t.columns.width(c), h };
    }, [row, col]);

  // ---- Alt+Down: a pick list of the column's own values ----
  await start(5, 3);
  const cur0 = await p.evaluate(() => window.__nde.tab.cache.get(5, 3));
  await key("Alt+ArrowDown");
  await p.waitForTimeout(800);
  const picks = await p.locator(".picks .pick").allTextContents();
  ok(picks.length > 2 && picks.includes("Osaka"), `Alt+Down lists the column's values (${picks.slice(0, 5).join(", ")}…)`);
  if (out) await p.screenshot({ path: path.join(out, "pick-values.png"), clip: { x: 0, y: 0, width: 1000, height: 520 } });
  await key("ArrowDown");
  await key("ArrowDown");
  const chosen = picks[Math.min(picks.length - 1, picks.indexOf(cur0) + 2)];
  await key("Enter");
  ok((await p.evaluate(() => window.__nde.tab.cache.get(5, 3))) === chosen, `picking one puts it in the cell (${chosen})`);

  // ---- Double-click on the selection's bottom border jumps down ----
  await start(5, 1);
  let bx = await box(5, 1);
  await p.mouse.dblclick(bx.x + bx.w / 2, bx.y + bx.h - 1);
  await p.waitForTimeout(600);
  let s = await sel();
  ok(s.cell.row > 1000 && s.cell.col === 1 && !s.editing, `double-clicking the bottom border jumps to the end of the data (row ${s.cell.row})`);
  await start(5, 1);
  bx = await box(5, 1);
  await p.mouse.dblclick(bx.x + bx.w / 2, bx.y + bx.h / 2);
  await p.waitForTimeout(400);
  s = await sel();
  ok(s.editing, "double-clicking the middle of the cell still edits it");
  await key("Escape");

  // ---- Sort arrow ----
  await start(5, 2);
  await command("Sort A");
  await p.waitForTimeout(1500);
  ok(await p.evaluate(() => window.__nde.tab.sortArrow(2) === "up" && window.__nde.tab.sortArrow(1) === null), "the sorted column shows an up arrow");
  if (out) await p.screenshot({ path: path.join(out, "sort-arrow.png"), clip: { x: 0, y: 160, width: 1000, height: 120 } });
  await start(30, 4);
  await key("Control+z");
  await p.waitForTimeout(800);
  ok(await p.evaluate(() => window.__nde.tab.sortArrow(2) === null), "undoing the sort takes the arrow away");
  s = await sel();
  ok(s.cell.row === 30 && s.cell.col === 4, `undoing a sort leaves the selection where it is (R${s.cell.row + 1} C${s.cell.col + 1})`);

  // ---- Right-click menus say how many ----
  await p.evaluate(() => { window.__nde.tab.scrollY = 0; window.__nde.tab.bump(); });
  await start(4, 1, 6, 2);
  bx = await box(5, 1);
  await p.mouse.click(bx.x + 10, bx.y + 5, { button: "right" });
  await p.waitForTimeout(300);
  const items = await p.locator(".menu.context .label").allTextContents();
  ok(items.includes("Insert 3 rows above") && items.includes("Delete rows 5–7"), `the cell menu says how many (${items.filter((x) => /rows/.test(x)).join(" | ")})`);
  ok(items.includes("Edit cell") && items.some((x) => /Note and flag/.test(x)) && items.some((x) => /Pick from a list/.test(x)), "the cell menu has Edit cell, Note and Pick from a list");
  await key("Escape");

  // ---- Fill handle dragged back clears ----
  await start(10, 4, 13, 4);
  const before = await p.evaluate(() => [10, 11, 12, 13].map((r) => window.__nde.tab.cache.get(r, 4)));
  bx = await box(13, 4);
  await p.mouse.move(bx.x + bx.w - 3, bx.y + bx.h - 3);
  await p.mouse.down();
  const up = await box(11, 4);
  await p.mouse.move(up.x + up.w - 10, up.y + up.h / 2, { steps: 6 });
  await p.mouse.up();
  await p.waitForTimeout(600);
  const after = await p.evaluate(() => [10, 11, 12, 13].map((r) => window.__nde.tab.cache.get(r, 4)));
  ok(after[0] === before[0] && after[1] === before[1] && after[2] === "" && after[3] === "", `dragging the fill handle back clears the cells passed over (${after.join(",")})`);
  s = await sel();
  ok(s.rect.rows === 2, "and the selection shrinks to what is left");
  await p.evaluate(() => window.__nde.grid.focus());
  await key("Control+z");

  // ---- Cell bar: Enter applies and moves down ----
  await start(20, 2);
  await p.fill("textarea.cellvalue", "from the bar");
  await p.press("textarea.cellvalue", "Enter");
  await p.waitForTimeout(500);
  s = await sel();
  ok((await p.evaluate(() => window.__nde.tab.cache.get(20, 2))) === "from the bar" && s.cell.row === 21, "Enter in the cell bar puts the value in and moves down");

  // ---- Heading arrow with filter buttons off ----
  await p.evaluate(() => window.__nde.grid.focus());
  await key("Control+Shift+L");
  const off = await p.evaluate(() => !window.__nde.tab.filterButtons);
  if (off) {
    const hp = await p.evaluate(() => window.__nde.grid.headerPoint(3));
    await p.mouse.move(hp.x - 60, hp.y - 10);
    await p.waitForTimeout(200);
    await p.mouse.move(hp.x + 6, hp.y - 12);
    await p.waitForTimeout(200);
    await p.mouse.click(hp.x + 6, hp.y - 12);
    await p.waitForTimeout(300);
    ok(await p.isVisible(".menu.context :text('Column summary')"), "the heading arrow opens the column menu (with Column summary)");
    await key("Escape");
    await p.evaluate(() => window.__nde.grid.focus());
    await key("Control+Shift+L");
  } else ok(false, "Ctrl+Shift+L turned the filter buttons off");

  // ---- Link title ----
  await put(25, 6, "https://example.com/page");
  async function put(row, col, text) {
    await start(row, col);
    await p.keyboard.type(text);
    await p.keyboard.press("Enter");
    await p.waitForTimeout(300);
  }
  bx = await box(25, 6);
  await p.mouse.move(bx.x + 20, bx.y + bx.h / 2);
  await p.waitForTimeout(200);
  const title = await p.evaluate(() => [...document.querySelectorAll("canvas")].map((c) => c.getAttribute("title")).find(Boolean) ?? "");
  ok(/example\.com/.test(title || "") && /Ctrl\+click/.test(title || ""), `pointing at a link shows its address (${(title || "").replace(/\n/g, " / ")})`);

  ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
  await b.close();
  console.log(failed ? `${failed} FAILED` : "ALL PASS");
  process.exit(failed ? 1 : 0);
})();
